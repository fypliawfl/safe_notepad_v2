use either::Either;
use generic_array::GenericArray;
use msg::{
    AesKey, EncryptedActionRequest, EncryptedData, EncryptedPaste, GreetRequest, Msg, RsaPublicKey,
};
use rand::{rngs::ThreadRng, thread_rng, CryptoRng, Rng, RngCore};
use std::{
    array,
    collections::HashMap,
    time::{Duration, Instant},
};

const SESSION_KEY_LIFETIME: Duration = Duration::from_secs(120 * 60);

#[derive(Debug)]
pub struct State {
    rng: ThreadRng,
    // (old..=fresh session keys, public key, instant when fresh session key was created)
    session_and_rsa_keys: Vec<(Vec<AesKey>, RsaPublicKey, Instant)>,
    msgs: Vec<(gist::FileKey, Msg)>,
    // (name, content)
    pastes: HashMap<EncryptedData, EncryptedData>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            rng: thread_rng(),
            session_and_rsa_keys: Default::default(),
            msgs: Default::default(),
            pastes: Default::default(),
        }
    }
}

fn random_session_key<R: CryptoRng + RngCore>(rng: &mut R) -> AesKey {
    GenericArray::from(array::from_fn(|_| rng.gen()))
}

impl State {
    fn remove_paste(&mut self, name: &EncryptedData) -> anyhow::Result<()> {
        if self.pastes.remove(name).is_some() {
            for file_key in self.msgs.iter().filter_map(|msg| {
                msg.1
                    .as_encrypted_action_request()
                    .map(|request| (request.name() == name).then_some(&msg.0))
                    .flatten()
                    .or_else(|| {
                        msg.1
                            .as_encrypted_action_response()
                            .map(|(request, _)| (request.name() == name).then_some(&msg.0))
                            .flatten()
                    })
            }) {
                gist::remove(*file_key)?;
            }
        }
        Ok(())
    }

    fn new_paste(
        &mut self,
        encrypted_request: EncryptedActionRequest,
        paste: EncryptedPaste,
    ) -> anyhow::Result<()> {
        assert!(self.pastes.insert(paste.name, paste.content).is_none());
        gist::insert(&Msg::EncryptedActionResponse(
            encrypted_request.to_response(Either::Left(None)),
        ))
        .map(drop)
    }

    fn drain_requests(&mut self) -> anyhow::Result<()> {
        'a: for msg_index in (0..self.msgs.len()).rev() {
            if let Some(request) = self.msgs[msg_index].1.as_greet_request() {
                if self
                    .msgs
                    .iter()
                    .filter_map(|msg| msg.1.as_greet_response())
                    .all(|response| &response.0 != request)
                {
                    let key = random_session_key(&mut self.rng);
                    gist::insert(&Msg::GreetResponse(
                        request.clone().to_response(&mut self.rng, &key)?,
                    ))
                    .unwrap();
                    let rsa_public_key = self.msgs.remove(msg_index).1.greet_request().unwrap().0;
                    self.session_and_rsa_keys
                        .push((vec![key], rsa_public_key, Instant::now()));
                }
            } else if self.msgs[msg_index]
                .1
                .as_encrypted_action_request()
                .is_some()
            {
                let (file_key, msg) = self.msgs.remove(msg_index);
                let encrypted_request = msg.encrypted_action_request().unwrap();

                if self
                    .msgs
                    .iter()
                    .filter_map(|msg| msg.1.as_encrypted_action_response())
                    .all(|response| response.0 != encrypted_request)
                {
                    for (session_keys, rsa_public_key, last_session_key_creation_instant) in
                        self.session_and_rsa_keys.iter_mut()
                    {
                        for session_key_index in 0..session_keys.len() {
                            if encrypted_request
                                .clone()
                                .decrypt(&session_keys[session_key_index])
                                .is_ok()
                            {
                                if session_key_index == session_keys.len().saturating_sub(1) {
                                    if last_session_key_creation_instant.elapsed()
                                        >= SESSION_KEY_LIFETIME
                                    {
                                        session_keys.push(random_session_key(&mut self.rng));
                                    }
                                    if session_key_index != session_keys.len().saturating_sub(1) {
                                        gist::insert(&Msg::EncryptedActionResponse(
                                            encrypted_request.to_response(Either::Right(
                                                GreetRequest(rsa_public_key.clone())
                                                    .to_response(
                                                        &mut self.rng,
                                                        &session_keys.last().unwrap(),
                                                    )?
                                                    .1,
                                            )),
                                        ))?;
                                        continue 'a;
                                    }
                                }
                                match encrypted_request.clone() {
                                    EncryptedActionRequest::Get { name } => {
                                        if let Some(content) = self.pastes.get(&name) {
                                            gist::insert(&Msg::EncryptedActionResponse(
                                                encrypted_request.to_response(Either::Left(Some(
                                                    EncryptedPaste {
                                                        name,
                                                        content: content.clone(),
                                                    },
                                                ))),
                                            ))?;
                                        }
                                    }
                                    EncryptedActionRequest::Remove { name } => {
                                        self.remove_paste(&name)?;
                                        gist::remove(file_key)?;
                                    }
                                    EncryptedActionRequest::New(encrypted_paste) => {
                                        if !self.pastes.contains_key(&encrypted_paste.name) {
                                            self.new_paste(encrypted_request, encrypted_paste)?;
                                            gist::remove(file_key)?;
                                        }
                                    }
                                    EncryptedActionRequest::Mut(encrypted_paste) => {
                                        self.remove_paste(&encrypted_paste.name)?;
                                        self.new_paste(encrypted_request, encrypted_paste)?;
                                        gist::remove(file_key)?;
                                    }
                                }
                                continue 'a;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn main() {
    let mut state = State::default();

    loop {
        state.msgs = gist::collect().unwrap();
        state.drain_requests().unwrap();
    }
}
