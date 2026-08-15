//! [`HttpDirectoryClient`]: a second, real implementation of
//! `ankai_core::directory::DirectoryService` — the first being
//! `ankai_core::directory::InMemoryDirectory` — that talks to a real
//! `crate::app::router` server over genuine HTTP, signing publish requests
//! with the device's actual `SignatureKeyPair` per
//! `docs/adr/0008-identity-discovery-service.md`'s auth model.
//!
//! One instance is scoped to one device: it's constructed with that
//! device's id and real signing key, and every publish call signs as that
//! device. The `DirectoryService` trait's methods still take an explicit
//! `device: &DeviceId` parameter (unchanged from `core::directory`, since
//! this crate doesn't get to unilaterally redesign that trait) — this
//! client rejects any call naming a different device than the one it was
//! constructed for, rather than silently trying (and failing, since it
//! doesn't hold that device's key) to sign on its behalf.
//!
//! [`HttpDirectoryClient::claim_username`]/[`HttpDirectoryClient::lookup_username`]
//! are added as inherent methods, not `DirectoryService` trait methods —
//! usernames (see `crate::username`) are a directory-server-specific
//! concept `core::directory::DirectoryService` was never shaped around, and
//! this crate doesn't get to unilaterally extend that trait either.
//! `claim_username` signs exactly like `publish_key_package`/
//! `publish_endpoint_addr` do; `lookup_username` is an unauthenticated GET,
//! matching `key_packages`/`endpoint_addr`'s existing open-read shape.

use ankai_core::directory::DirectoryService;
use ankai_core::identity::DeviceId;
use ankai_core::util::encode_hex;
use ankai_core::Error;
use iroh::EndpointAddr;
use openmls::key_packages::KeyPackage;
use openmls_basic_credential::SignatureKeyPair;
use openmls_traits::signatures::Signer;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::{auth, now_unix, paths};

pub struct HttpDirectoryClient {
    base_url: String,
    http: reqwest::Client,
    device_id: DeviceId,
    pubkey_hex: String,
    signer: SignatureKeyPair,
}

impl HttpDirectoryClient {
    /// `base_url` should have no trailing slash (e.g. `http://127.0.0.1:7420`).
    /// `signer` must be the real `SignatureKeyPair` backing `device_id` —
    /// e.g. read back from `AnkaiMlsProvider` storage the same way
    /// `ankai_core::identity::create_key_package` does.
    pub fn new(base_url: impl Into<String>, device_id: DeviceId, signer: SignatureKeyPair) -> Self {
        let pubkey_hex = encode_hex(signer.public());
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::new(),
            device_id,
            pubkey_hex,
            signer,
        }
    }

    fn signed_headers(&self, method: &str, path: &str, body: &[u8]) -> Result<HeaderMap, Error> {
        let timestamp = now_unix();
        let msg = auth::signing_message(method, path, &self.device_id.0, timestamp, body);
        let signature = self
            .signer
            .sign(&msg)
            .map_err(|e| Error::Directory(format!("failed to sign directory request: {e:?}")))?;

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static(auth::DEVICE_PUBKEY_HEADER),
            HeaderValue::from_str(&self.pubkey_hex)
                .map_err(|e| Error::Directory(format!("invalid pubkey header: {e}")))?,
        );
        headers.insert(
            HeaderName::from_static(auth::TIMESTAMP_HEADER),
            HeaderValue::from_str(&timestamp.to_string())
                .map_err(|e| Error::Directory(format!("invalid timestamp header: {e}")))?,
        );
        headers.insert(
            HeaderName::from_static(auth::SIGNATURE_HEADER),
            HeaderValue::from_str(&encode_hex(&signature))
                .map_err(|e| Error::Directory(format!("invalid signature header: {e}")))?,
        );
        Ok(headers)
    }

    fn require_own_device(&self, device: &DeviceId) -> Result<(), Error> {
        if device != &self.device_id {
            return Err(Error::Directory(
                "HttpDirectoryClient can only publish on behalf of the device it was constructed for"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

impl DirectoryService for HttpDirectoryClient {
    async fn publish_key_package(
        &self,
        device: &DeviceId,
        key_package: KeyPackage,
    ) -> Result<(), Error> {
        self.require_own_device(device)?;
        let path = paths::key_packages_path(&device.0);
        let body = serde_json::to_vec(&key_package)
            .map_err(|e| Error::Directory(format!("failed to encode key package: {e}")))?;
        let headers = self.signed_headers("POST", &path, &body)?;

        let resp = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|e| Error::Directory(format!("publish_key_package request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(Error::Directory(format!(
                "publish_key_package rejected: {}",
                resp.status()
            )));
        }
        Ok(())
    }

    async fn key_packages(&self, device: &DeviceId) -> Result<Vec<KeyPackage>, Error> {
        let path = paths::key_packages_path(&device.0);
        let resp = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .send()
            .await
            .map_err(|e| Error::Directory(format!("key_packages request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(Error::Directory(format!(
                "key_packages rejected: {}",
                resp.status()
            )));
        }
        resp.json()
            .await
            .map_err(|e| Error::Directory(format!("failed to decode key_packages response: {e}")))
    }

    async fn publish_endpoint_addr(
        &self,
        device: &DeviceId,
        addr: EndpointAddr,
    ) -> Result<(), Error> {
        self.require_own_device(device)?;
        let path = paths::endpoint_addr_path(&device.0);
        let body = serde_json::to_vec(&addr)
            .map_err(|e| Error::Directory(format!("failed to encode endpoint addr: {e}")))?;
        let headers = self.signed_headers("POST", &path, &body)?;

        let resp = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|e| Error::Directory(format!("publish_endpoint_addr request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(Error::Directory(format!(
                "publish_endpoint_addr rejected: {}",
                resp.status()
            )));
        }
        Ok(())
    }

    async fn endpoint_addr(&self, device: &DeviceId) -> Result<Option<EndpointAddr>, Error> {
        let path = paths::endpoint_addr_path(&device.0);
        let resp = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .send()
            .await
            .map_err(|e| Error::Directory(format!("endpoint_addr request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(Error::Directory(format!(
                "endpoint_addr rejected: {}",
                resp.status()
            )));
        }
        resp.json()
            .await
            .map_err(|e| Error::Directory(format!("failed to decode endpoint_addr response: {e}")))
    }
}

impl HttpDirectoryClient {
    /// Claims (or updates) `device`'s username against the real directory
    /// server, signed exactly like `publish_key_package`/
    /// `publish_endpoint_addr`. See `crate::username`'s module doc comment
    /// for the validation rule and first-come-first-served/same-device-can-
    /// update semantics this call is subject to server-side; a rejection
    /// (bad format, or already claimed by a different device) comes back
    /// as an `Err` whose message includes the server's real response body,
    /// not just a bare status code.
    pub async fn claim_username(&self, device: &DeviceId, username: &str) -> Result<(), Error> {
        self.require_own_device(device)?;
        let path = paths::username_path(&device.0);

        #[derive(serde::Serialize)]
        struct ClaimUsernameRequest<'a> {
            username: &'a str,
        }
        let body = serde_json::to_vec(&ClaimUsernameRequest { username })
            .map_err(|e| Error::Directory(format!("failed to encode username claim: {e}")))?;
        let headers = self.signed_headers("POST", &path, &body)?;

        let resp = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|e| Error::Directory(format!("claim_username request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            return Err(Error::Directory(format!(
                "claim_username rejected: {status} {detail}"
            )));
        }
        Ok(())
    }

    /// Looks up which `DeviceId` currently claims `username`, or `None` if
    /// nobody does. Unauthenticated (a plain `GET`), matching
    /// `key_packages`/`endpoint_addr`'s existing open-read semantics — see
    /// `crate::app`'s module doc comment.
    pub async fn lookup_username(&self, username: &str) -> Result<Option<DeviceId>, Error> {
        let path = paths::username_lookup_path(username);
        let resp = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .send()
            .await
            .map_err(|e| Error::Directory(format!("lookup_username request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(Error::Directory(format!(
                "lookup_username rejected: {}",
                resp.status()
            )));
        }

        let device_id: Option<String> = resp.json().await.map_err(|e| {
            Error::Directory(format!("failed to decode lookup_username response: {e}"))
        })?;
        Ok(device_id.map(DeviceId))
    }
}
