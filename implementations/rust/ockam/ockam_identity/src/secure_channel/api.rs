use ockam_core::compat::vec::Vec;
use ockam_core::Error;
use ockam_core::Message;
use serde::{Deserialize, Serialize};

/// Request type for `EncryptorWorker` API Address
#[derive(Serialize, Deserialize, Message)]
pub enum EncryptionRequest {
    /// Encrypt data
    Encrypt(Vec<u8>),
    /// Trigger a manual rekey
    Rekey,
    /// Derive new key
    DeriveNewKey,
}

/// Response type for `EncryptorWorker` API Address
#[derive(Serialize, Deserialize, Message)]
pub enum EncryptionResponse {
    /// Success
    Ok(Vec<u8>),
    /// Error
    Err(Error),
}

/// Request type for `Decryptor` API Address (the `Decryptor` is accessible through the `HandshakeWorker`)
#[derive(Serialize, Deserialize, Message)]
pub enum DecryptionRequest {
    /// Decrypt data
    Decrypt {
        /// Ciphertext to decrypt
        ciphertext: Vec<u8>,
        /// Rekey counter
        rekey_counter: Option<u16>,
    },
    /// Derive new key
    DeriveNewKey,
}

/// Response type for `Decryptor` API Address (the `Decryptor` is accessible through the `HandshakeWorker`)
#[derive(Serialize, Deserialize, Message)]
pub enum DecryptionResponse {
    /// Success
    Ok(Vec<u8>),
    /// Error
    Err(Error),
}
