//! Local-first persistence (SQLite). See docs/threat-model.md "Local data" —
//! message history and other sensitive local state must be encrypted at
//! rest, keyed to OS secure storage. Encryption-at-rest wiring lands
//! alongside docs/adr/0004-e2ee-stack.md; this module is currently an
//! empty placeholder so `core` compiles as a workspace member.
