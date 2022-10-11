use either::Either;
pub use rsa::{RsaPrivateKey, RsaPublicKey};

use aes::{
    cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit},
    Aes256,
};
use generic_array::GenericArray;
use rand::{CryptoRng, RngCore};
use rsa::{errors::Result as RsaResult, PaddingScheme, PublicKey};
use serde::{Deserialize, Serialize};
use serde_encrypt::{serialize::impls::BincodeSerializer, traits::SerdeEncryptSharedKey};
use typenum::consts::U16;

pub type AesKey = aes::cipher::Key<Aes256>;

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct GreetRequest(pub RsaPublicKey);

impl GreetRequest {
    pub fn to_response<R: CryptoRng + RngCore>(
        self,
        rng: &mut R,
        key: &AesKey,
    ) -> RsaResult<GreetResponse> {
        self.0
            .encrypt(rng, PaddingScheme::new_pkcs1v15_encrypt(), key)
            .map(|bytes| (self, bytes))
    }
}

pub type EncryptedAesKey = Vec<u8>;

pub type GreetResponse = (GreetRequest, EncryptedAesKey);

pub fn decrypt_aes_key(
    aes_key: &EncryptedAesKey,
    rsa_private_key: &RsaPrivateKey,
) -> RsaResult<AesKey> {
    rsa_private_key
        .decrypt(PaddingScheme::new_pkcs1v15_encrypt(), aes_key)
        .map(|bytes| GenericArray::from_slice(&bytes).clone())
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct EncryptedData {
    pub content: Vec<GenericArray<u8, U16>>,
    pub zero_suffix_len: usize,
}

impl EncryptedData {
    pub fn encrypt<T: Serialize>(x: &T, key: &AesKey) -> serde_cbor::Result<Self> {
        let bytes = serde_cbor::to_vec(x)?;
        let zero_suffix_len = bytes.len() % 16;
        let mut content: Vec<_> = bytes
            .chunks(16)
            .map(|chunk| {
                if chunk.len() == 16 {
                    GenericArray::from_slice(chunk).clone()
                } else {
                    let mut array = [0; 16];
                    array[0..chunk.len()].copy_from_slice(chunk);
                    GenericArray::from_slice(&array).clone()
                }
            })
            .collect();
        let mut cipher = Aes256::new(key);
        cipher.encrypt_blocks_mut(&mut content);
        Ok(Self {
            content,
            zero_suffix_len,
        })
    }

    pub fn decrypt<T: for<'de> Deserialize<'de>>(mut self, key: &AesKey) -> serde_cbor::Result<T> {
        let mut cipher = Aes256::new(key);
        cipher.decrypt_blocks_mut(&mut self.content);
        let mut bytes = Vec::with_capacity(self.content.len());
        for chunk in &self.content[..self.content.len().saturating_sub(1)] {
            bytes.extend_from_slice(chunk);
        }
        if let Some(last) = self.content.last() {
            bytes.extend_from_slice(&last[..16usize.saturating_sub(self.zero_suffix_len)]);
        }
        serde_cbor::from_slice(&bytes)
    }
}

impl SerdeEncryptSharedKey for EncryptedData {
    type S = BincodeSerializer<Self>;
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Paste {
    pub name: String,
    pub content: String,
}

impl Paste {
    pub fn encrypt(&self, key: &AesKey) -> serde_cbor::Result<EncryptedPaste> {
        Ok(EncryptedPaste {
            name: EncryptedData::encrypt(&self.name, key)?,
            content: EncryptedData::encrypt(&self.content, key)?,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct EncryptedPaste {
    pub name: EncryptedData,
    pub content: EncryptedData,
}

impl EncryptedPaste {
    pub fn decrypt_name(&self, key: &AesKey) -> serde_cbor::Result<String> {
        self.name.clone().decrypt(key)
    }

    pub fn decrypt_content(&self, key: &AesKey) -> serde_cbor::Result<String> {
        self.content.clone().decrypt(key)
    }

    pub fn decrypt(&self, key: &AesKey) -> serde_cbor::Result<Paste> {
        Ok(Paste {
            name: self.decrypt_name(key)?,
            content: self.decrypt_content(key)?,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ActionRequest {
    Get { name: String },
    Remove { name: String },
    Mut(Paste),
    New(Paste),
}

impl ActionRequest {
    pub fn encrypt(&self, key: &AesKey) -> serde_cbor::Result<EncryptedActionRequest> {
        Ok(match self {
            ActionRequest::Get { name } => EncryptedActionRequest::Get {
                name: EncryptedData::encrypt(&name, key)?,
            },
            ActionRequest::Remove { name } => EncryptedActionRequest::Remove {
                name: EncryptedData::encrypt(&name, key)?,
            },
            ActionRequest::Mut(paste) => EncryptedActionRequest::Mut(paste.encrypt(key)?),
            ActionRequest::New(paste) => EncryptedActionRequest::New(paste.encrypt(key)?),
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum EncryptedActionRequest {
    New(EncryptedPaste),
    Mut(EncryptedPaste),
    Get { name: EncryptedData },
    Remove { name: EncryptedData },
}

impl EncryptedActionRequest {
    pub fn decrypt(self, key: &AesKey) -> serde_cbor::Result<ActionRequest> {
        Ok(match self {
            EncryptedActionRequest::New(paste) => ActionRequest::New(paste.decrypt(key)?),
            EncryptedActionRequest::Mut(paste) => ActionRequest::Mut(paste.decrypt(key)?),
            EncryptedActionRequest::Get { name } => ActionRequest::Get {
                name: name.decrypt(key)?,
            },
            EncryptedActionRequest::Remove { name } => ActionRequest::Remove {
                name: name.decrypt(key)?,
            },
        })
    }

    pub fn to_response(
        self,
        payload: Either<Option<EncryptedPaste>, EncryptedAesKey>,
    ) -> EncryptedActionResponse {
        (self, payload)
    }

    pub fn name(&self) -> &EncryptedData {
        match self {
            EncryptedActionRequest::New(EncryptedPaste { name, .. }) => name,
            EncryptedActionRequest::Mut(EncryptedPaste { name, .. }) => name,
            EncryptedActionRequest::Get { name } => name,
            EncryptedActionRequest::Remove { name } => name,
        }
    }

    pub fn paste(&self) -> Option<&EncryptedPaste> {
        match self {
            EncryptedActionRequest::New(paste) => Some(paste),
            EncryptedActionRequest::Mut(paste) => Some(paste),
            EncryptedActionRequest::Get { .. } => None,
            EncryptedActionRequest::Remove { .. } => None,
        }
    }

    pub fn as_get(&self) -> Option<&EncryptedData> {
        if let Self::Get { name } = self {
            Some(name)
        } else {
            None
        }
    }

    pub fn as_remove(&self) -> Option<&EncryptedData> {
        if let Self::Remove { name } = self {
            Some(name)
        } else {
            None
        }
    }

    pub fn as_new(&self) -> Option<&EncryptedPaste> {
        if let Self::New(encrypted_paste) = self {
            Some(encrypted_paste)
        } else {
            None
        }
    }

    pub fn as_mut(&self) -> Option<&EncryptedPaste> {
        if let Self::Mut(encrypted_paste) = self {
            Some(encrypted_paste)
        } else {
            None
        }
    }
}

pub type EncryptedActionResponse = (
    EncryptedActionRequest,
    Either<Option<EncryptedPaste>, EncryptedAesKey>,
);

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum Msg {
    GreetRequest(GreetRequest),
    GreetResponse(GreetResponse),
    EncryptedActionRequest(EncryptedActionRequest),
    EncryptedActionResponse(EncryptedActionResponse),
}

impl Msg {
    pub fn as_greet_request(&self) -> Option<&GreetRequest> {
        if let Self::GreetRequest(request) = self {
            Some(request)
        } else {
            None
        }
    }

    pub fn as_greet_response(&self) -> Option<&GreetResponse> {
        if let Self::GreetResponse(response) = self {
            Some(response)
        } else {
            None
        }
    }

    pub fn as_encrypted_action_request(&self) -> Option<&EncryptedActionRequest> {
        if let Self::EncryptedActionRequest(request) = self {
            Some(request)
        } else {
            None
        }
    }

    pub fn as_encrypted_action_response(&self) -> Option<&EncryptedActionResponse> {
        if let Self::EncryptedActionResponse(response) = self {
            Some(response)
        } else {
            None
        }
    }

    pub fn greet_request(self) -> Option<GreetRequest> {
        if let Self::GreetRequest(request) = self {
            Some(request)
        } else {
            None
        }
    }

    pub fn greet_response(self) -> Option<GreetResponse> {
        if let Self::GreetResponse(response) = self {
            Some(response)
        } else {
            None
        }
    }

    pub fn encrypted_action_request(self) -> Option<EncryptedActionRequest> {
        if let Self::EncryptedActionRequest(request) = self {
            Some(request)
        } else {
            None
        }
    }

    pub fn encrypted_action_response(self) -> Option<EncryptedActionResponse> {
        if let Self::EncryptedActionResponse(response) = self {
            Some(response)
        } else {
            None
        }
    }
}
