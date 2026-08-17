//! [`AccountsClient`]: a real network client for `crate::app::router`,
//! signing requests with a real `ankai_core::account::AccountRootKeyPair` —
//! mirroring `server/directory`'s `HttpDirectoryClient` shape (a plain
//! struct with inherent async methods, not a trait implementation, since
//! `docs/adr/0009-account-system.md`'s account model has no pre-existing
//! `core` trait the way `core::directory::DirectoryService` already existed
//! for the directory service).
//!
//! One instance is scoped to one account: constructed with that account's
//! real `AccountRootKeyPair`, every signed call authenticates as that
//! account. `add_device`/`revoke_device`/recovery-blob calls all operate on
//! `self`'s own account id — there's no "publish on behalf of a different
//! account" footgun to guard against here the way `HttpDirectoryClient`
//! does for `DeviceId`, because every signed message this client produces
//! embeds `self`'s own derived account id directly (see
//! `ankai_core::account::derive_account_id`), so presenting it against a
//! mismatched URL path is simply a signature/self-certification failure on
//! the server side, not something this client needs to pre-check.

use ankai_core::account::{sign_device_registration, AccountRootKeyPair};
use ankai_core::identity::DeviceId;
use ankai_core::util::encode_hex;
use ankai_core::Error;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::db::DeviceRegistrationRow;
use crate::{auth, now_unix, paths};

pub struct AccountsClient {
    base_url: String,
    http: reqwest::Client,
    account_key: AccountRootKeyPair,
}

/// The result of [`AccountsClient::list_devices`]: the account's public key
/// (so a caller can independently verify each registration themselves via
/// `ankai_core::account::verify_device_registration`, rather than trusting
/// this client's — or the server's — word for it) plus the raw device rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceList {
    pub account_public_key: Vec<u8>,
    pub devices: Vec<DeviceRegistrationRow>,
}

impl AccountsClient {
    /// `base_url` should have no trailing slash (e.g. `http://127.0.0.1:7421`).
    pub fn new(base_url: impl Into<String>, account_key: AccountRootKeyPair) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::new(),
            account_key,
        }
    }

    pub fn account_id(&self) -> String {
        self.account_key.identity().id.0
    }

    fn envelope_headers(&self, method: &str, path: &str, body: &[u8]) -> Result<HeaderMap, Error> {
        let timestamp = now_unix();
        let account_id = self.account_id();
        let msg = auth::signing_message(method, path, &account_id, timestamp, body);
        let signature = self.account_key.sign(&msg);

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static(auth::ACCOUNT_PUBKEY_HEADER),
            HeaderValue::from_str(&encode_hex(&self.account_key.identity().public_key))
                .map_err(|e| Error::Account(format!("invalid pubkey header: {e}")))?,
        );
        headers.insert(
            HeaderName::from_static(auth::TIMESTAMP_HEADER),
            HeaderValue::from_str(&timestamp.to_string())
                .map_err(|e| Error::Account(format!("invalid timestamp header: {e}")))?,
        );
        headers.insert(
            HeaderName::from_static(auth::SIGNATURE_HEADER),
            HeaderValue::from_str(&encode_hex(&signature))
                .map_err(|e| Error::Account(format!("invalid signature header: {e}")))?,
        );
        Ok(headers)
    }

    /// Signs a fresh `DeviceRegistration` for `device`/`device_public_key`
    /// and adds/refreshes it against the server — the same call whether
    /// this is the account's very first device (implicitly creating the
    /// account server-side) or a later one. See `crate::app::add_device`'s
    /// doc comment for why there's no separate "create account" step.
    pub async fn add_device(
        &self,
        device: DeviceId,
        device_public_key: Vec<u8>,
    ) -> Result<(), Error> {
        let registration =
            sign_device_registration(&self.account_key, device, device_public_key, now_unix());

        #[derive(serde::Serialize)]
        struct AddDeviceRequest<'a> {
            account_public_key_hex: String,
            registration: &'a ankai_core::account::DeviceRegistration,
        }
        let body = serde_json::to_vec(&AddDeviceRequest {
            account_public_key_hex: encode_hex(&self.account_key.identity().public_key),
            registration: &registration,
        })
        .map_err(|e| Error::Account(format!("failed to encode device registration: {e}")))?;

        let path = paths::devices_path(&self.account_id());
        let resp = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .body(body)
            .send()
            .await
            .map_err(|e| Error::Account(format!("add_device request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            return Err(Error::Account(format!(
                "add_device rejected: {status} {detail}"
            )));
        }
        Ok(())
    }

    /// Lists the current (non-revoked) device registrations for `account_id`
    /// — a plain, unauthenticated read, so this can look up *any* account,
    /// not just `self`'s own (mirroring
    /// `docs/adr/0008-identity-discovery-service.md`'s open-read precedent).
    /// `Ok(None)` if `account_id` has never had a device registered.
    pub async fn list_devices(&self, account_id: &str) -> Result<Option<DeviceList>, Error> {
        let path = paths::devices_path(account_id);
        let resp = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .send()
            .await
            .map_err(|e| Error::Account(format!("list_devices request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(Error::Account(format!(
                "list_devices rejected: {}",
                resp.status()
            )));
        }

        #[derive(serde::Deserialize)]
        struct ListDevicesResponse {
            account_public_key_hex: String,
            devices: Vec<DeviceRegistrationRow>,
        }
        let parsed: ListDevicesResponse = resp
            .json()
            .await
            .map_err(|e| Error::Account(format!("failed to decode list_devices response: {e}")))?;
        let account_public_key = ankai_core::util::decode_hex(&parsed.account_public_key_hex)
            .ok_or_else(|| {
                Error::Account("server returned non-hex account public key".to_string())
            })?;

        Ok(Some(DeviceList {
            account_public_key,
            devices: parsed.devices,
        }))
    }

    /// Revokes `device` from `self`'s own account.
    pub async fn revoke_device(&self, device: &DeviceId) -> Result<(), Error> {
        let account_id = self.account_id();
        let path = paths::revoke_path(&account_id, &device.0);
        let headers = self.envelope_headers("POST", &path, &[])?;

        let resp = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .headers(headers)
            .send()
            .await
            .map_err(|e| Error::Account(format!("revoke_device request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(Error::Account(format!(
                "revoke_device rejected: {}",
                resp.status()
            )));
        }
        Ok(())
    }

    /// Uploads `blob` as `self`'s opaque recovery backup, replacing any
    /// previous one. `blob` must already be client-side ciphertext — see
    /// `docs/adr/0004-e2ee-stack.md`; this client has no opinion on its
    /// contents.
    pub async fn put_recovery_blob(&self, blob: Vec<u8>) -> Result<(), Error> {
        let account_id = self.account_id();
        let path = paths::recovery_blob_path(&account_id);
        let headers = self.envelope_headers("PUT", &path, &blob)?;

        let resp = self
            .http
            .put(format!("{}{}", self.base_url, path))
            .headers(headers)
            .body(blob)
            .send()
            .await
            .map_err(|e| Error::Account(format!("put_recovery_blob request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            return Err(Error::Account(format!(
                "put_recovery_blob rejected: {status} {detail}"
            )));
        }
        Ok(())
    }

    /// Fetches `self`'s currently stored recovery blob, or `None` if
    /// nothing has ever been uploaded.
    pub async fn get_recovery_blob(&self) -> Result<Option<Vec<u8>>, Error> {
        let account_id = self.account_id();
        let path = paths::recovery_blob_path(&account_id);
        let headers = self.envelope_headers("GET", &path, &[])?;

        let resp = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .headers(headers)
            .send()
            .await
            .map_err(|e| Error::Account(format!("get_recovery_blob request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(Error::Account(format!(
                "get_recovery_blob rejected: {}",
                resp.status()
            )));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| Error::Account(format!("failed to read recovery blob response: {e}")))?;
        Ok(Some(bytes.to_vec()))
    }
}
