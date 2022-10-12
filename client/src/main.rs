use std::{
    fs,
    time::{Duration, Instant},
};

use eframe::{
    egui::{
        Button, CentralPanel, FontData, FontDefinitions, FontTweak, Label, TextBuffer, TextEdit,
        TextStyle, Ui,
    },
    epaint::{FontFamily, Vec2},
};
use msg::{
    decrypt_aes_key, ActionRequest, AesKey, EncryptedActionRequest, GreetRequest, Msg, Paste,
    RsaPrivateKey,
};
use rand::{rngs::ThreadRng, thread_rng, CryptoRng, RngCore};

const RSA_PRIVATE_KEY_FILE_NAME: &'static str = "rsa_private_key.json";

fn generate_rsa_private_key<R: CryptoRng + RngCore>(rng: &mut R) -> RsaPrivateKey {
    let key = RsaPrivateKey::new(rng, 1024).unwrap();
    fs::write(
        RSA_PRIVATE_KEY_FILE_NAME,
        serde_json::to_vec_pretty(&key).unwrap(),
    )
    .unwrap();
    key
}

fn rsa_private_key<R: CryptoRng + RngCore>(rng: &mut R) -> RsaPrivateKey {
    let read = fs::read(RSA_PRIVATE_KEY_FILE_NAME)
        .ok()
        .map(|bytes| serde_json::from_slice(&bytes).ok())
        .flatten();
    read.unwrap_or_else(|| generate_rsa_private_key(rng))
}

struct App {
    rng: ThreadRng,
    session_key: Option<AesKey>,
    msgs: Vec<(gist::FileKey, Msg)>,
    pending_request_retry_instant: Instant,
    pending_get_request: Option<EncryptedActionRequest>,
    pending_get_request_start_instant: Instant,
    name: String,
    content: String,
}

const PENDING_REQUEST_RETRY_PERIOD: Duration = Duration::from_secs(3);

const PENDING_GET_REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

fn pending_label(ui: &mut Ui, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(text);
    });
}

fn available_width(ui: &Ui, style: &TextStyle) -> Vec2 {
    Vec2::new(ui.available_width(), ui.text_style_height(style))
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        let mut fonts = FontDefinitions::default();
        fonts.font_data.insert("stalinist".into(), {
            let mut font_data = FontData::from_static(include_bytes!("StalinistOne-Regular.ttf"));
            font_data.tweak = FontTweak {
                scale: 1.4,
                ..Default::default()
            };
            font_data
        });
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "stalinist".to_owned());
        cc.egui_ctx.set_fonts(fonts);

        let mut rng = thread_rng();
        let msgs = gist::collect()?;
        let rsa_private_key = rsa_private_key(&mut rng);
        let rsa_public_key = rsa_private_key.to_public_key();

        let session_key = if let Some((_, encrypted_session_key)) = msgs
            .iter()
            .flat_map(|(_, msg)| msg.as_greet_response())
            .find(|(request, _)| request.0 == rsa_public_key)
        {
            Some(decrypt_aes_key(&encrypted_session_key, &rsa_private_key)?)
        } else {
            dbg!(gist::insert(&msg::Msg::GreetRequest(GreetRequest(rsa_public_key)))?);
            None
        };

        Ok(Self {
            rng,
            session_key,
            msgs,
            pending_request_retry_instant: Instant::now(),
            pending_get_request_start_instant: Instant::now(),
            pending_get_request: None,
            name: "Имя новой записи".into(),
            content: "Содержание новой записи".into(),
        })
    }

    fn show_pending_greet_request(&mut self, ui: &mut Ui) -> anyhow::Result<()> {
        pending_label(ui, "Получаем сессионный ключ ...");
        if self.pending_request_retry_instant.elapsed() >= PENDING_REQUEST_RETRY_PERIOD {
            self.msgs = gist::collect()?;
            let rsa_private_key = rsa_private_key(&mut self.rng);
            let greet_request = GreetRequest(rsa_private_key.to_public_key());
            if let Some((_, encryted_session_key)) = self
                .msgs
                .iter()
                .filter_map(|(_, msg)| msg.as_greet_response())
                .find(|greet_response| greet_response.0 == greet_request)
            {
                self.session_key = Some(decrypt_aes_key(&encryted_session_key, &rsa_private_key)?);
            } else {
                self.pending_request_retry_instant = Instant::now();
            }
        }
        Ok(())
    }

    fn show_new_rsa_key(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.group(|ui| {
                if ui.button("Новый RSA ключ").clicked() {
                    generate_rsa_private_key(&mut self.rng);
                    self.session_key = None;
                    gist::insert(&Msg::GreetRequest(GreetRequest(
                        rsa_private_key(&mut self.rng).to_public_key(),
                    )))
                    .unwrap();
                };
                ui.add_sized(
                    available_width(ui, &TextStyle::Body),
                    Label::new(format!("{} ...", {
                        serde_json::to_string(&rsa_private_key(&mut self.rng))
                            .unwrap()
                            .trim_start_matches(|ch: char| !ch.is_numeric())
                            .char_range(0..30)
                    })),
                );
            });
        });
    }

    fn msgs_contain_encrypted_request(&self, encrypted_request: &EncryptedActionRequest) -> bool {
        self.msgs
            .iter()
            .filter_map(|(_, msg)| msg.as_encrypted_action_request())
            .find(|other_encrypted_request| *other_encrypted_request == encrypted_request)
            .is_some()
    }

    fn pastebin_insert_if_no_msg_contains_encrypted_request(
        &self,
        encrypted_request: EncryptedActionRequest,
    ) -> anyhow::Result<()> {
        if !self.msgs_contain_encrypted_request(&encrypted_request) {
            gist::insert(&Msg::EncryptedActionRequest(encrypted_request))?;
        }
        Ok(())
    }

    fn clone_paste(&self) -> Paste {
        Paste {
            name: self.name.clone(),
            content: self.content.clone(),
        }
    }

    fn show_actions(&mut self, ui: &mut Ui, session_key: &AesKey) {
        ui.horizontal(|ui| {
            if ui.button("Новая запись").clicked() {
                self.msgs = gist::collect().unwrap();
                let encrypted_request = ActionRequest::New(self.clone_paste())
                    .encrypt(session_key)
                    .unwrap();
                self.pastebin_insert_if_no_msg_contains_encrypted_request(encrypted_request)
                    .unwrap();
            }
            if ui.button("Редактировать запись").clicked() {
                self.msgs = gist::collect().unwrap();
                let encrypted_request = ActionRequest::Mut(self.clone_paste())
                    .encrypt(session_key)
                    .unwrap();
                self.pastebin_insert_if_no_msg_contains_encrypted_request(encrypted_request)
                    .unwrap();
            }
            if ui
                .add_sized(
                    available_width(&ui, &TextStyle::Button),
                    Button::new("Удалить запись"),
                )
                .clicked()
            {
                self.msgs = gist::collect().unwrap();
                let encrypted_request = ActionRequest::Remove {
                    name: self.name.clone(),
                }
                .encrypt(session_key)
                .unwrap();
                self.pastebin_insert_if_no_msg_contains_encrypted_request(encrypted_request)
                    .unwrap();
            }
        });
    }

    fn show_get_and_name(&mut self, ui: &mut Ui, session_key: &AesKey) {
        ui.horizontal(|ui| {
            if ui.button("Найти запись").clicked() {
                let encrypted_request_msg = Msg::EncryptedActionRequest(
                    ActionRequest::Get {
                        name: self.name.clone(),
                    }
                    .encrypt(session_key)
                    .unwrap(),
                );
                gist::insert(&encrypted_request_msg).unwrap();
                self.pending_get_request_start_instant = Instant::now();
                self.pending_get_request =
                    Some(encrypted_request_msg.encrypted_action_request().unwrap());
            }
            ui.add_sized(
                available_width(ui, &TextStyle::Body),
                TextEdit::singleline(&mut self.name),
            );
        });
    }

    fn show_pending_get_request(
        &mut self,
        ui: &mut Ui,
        session_key: &AesKey,
    ) -> anyhow::Result<()> {
        pending_label(ui, &format!("Получаем запись \"{}\" ...", self.name));
        if self.pending_get_request_start_instant.elapsed() < PENDING_GET_REQUEST_TIMEOUT {
            if self.pending_request_retry_instant.elapsed() >= PENDING_REQUEST_RETRY_PERIOD {
                self.msgs = gist::collect()?;
                let pending_get_request = self.pending_get_request.as_ref().unwrap();
                if let Some((file_key, (_, encrypted_response))) = self
                    .msgs
                    .iter()
                    .filter_map(|(api_paste_key, msg)| {
                        msg.as_encrypted_action_response()
                            .map(|encrypted_response| (api_paste_key, encrypted_response))
                    })
                    .find(|(_, encrypted_response)| &encrypted_response.0 == pending_get_request)
                {
                    match encrypted_response {
                        either::Either::Left(paste) => {
                            if let Some(paste) = paste.as_ref() {
                                self.name = paste.decrypt_name(session_key)?;
                                self.content = paste.decrypt_content(session_key)?;
                                self.pending_get_request = None;
                            } else {
                                anyhow::bail!("EncryptedActionRequest yielded no paste");
                            }
                        }
                        either::Either::Right(encrypted_session_key) => {
                            let rsa_private_key = rsa_private_key(&mut self.rng);
                            self.session_key =
                                Some(decrypt_aes_key(encrypted_session_key, &rsa_private_key)?);
                            gist::remove(*file_key)?;
                        }
                    }
                }
            }
        } else {
            self.pending_get_request = None;
        }

        Ok(())
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        CentralPanel::default().show(ctx, |ui| {
            if let Some(session_key) = self.session_key.clone() {
                if self.pending_get_request.is_some() {
                    self.show_pending_get_request(ui, &session_key).unwrap();
                } else {
                    self.show_new_rsa_key(ui);
                    ui.group(|ui| {
                        self.show_actions(ui, &session_key);
                        self.show_get_and_name(ui, &session_key);
                    });
                    ui.add_sized(ui.available_size(), TextEdit::multiline(&mut self.content));
                }
            } else {
                self.show_pending_greet_request(ui).unwrap();
            }
        });
        ctx.request_repaint_after(PENDING_REQUEST_RETRY_PERIOD + Duration::from_secs(1));
    }
}

fn main() {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "Защищённый блокнот",
        native_options,
        Box::new(|cc| Box::new(App::new(cc).unwrap())),
    );
}
