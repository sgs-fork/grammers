use chrono::Local;
use futures_util::future::{select, Either};
use grammers_client::session::Session;
use grammers_client::types::{chat, message};
use grammers_client::{types, Client, Config, InitParams, SignInError, Update};
use grammers_mtsender::{FixedReconnect, InvocationError, ReconnectionPolicy};
use grammers_session::PackedType;
use grammers_session::PackedType::Chat;
use log::{error, info, warn, LevelFilter};
use simple_logger::*;
use std::io::Write;
use std::io::{self, BufRead as _, Write as _};
use std::ops::ControlFlow;
use std::pin::pin;
use std::thread;
use std::time::Duration;
use std::time::SystemTime;
use tokio::{runtime, signal, task, time};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const SESSION_FILE: &str = "tdata";

struct AutoReconnectPolicy;

impl ReconnectionPolicy for AutoReconnectPolicy {
    fn should_retry(&self, attempts: usize) -> ControlFlow<(), Duration> {
        let duration = u64::pow(2, attempts as _);
        warn!("Auto-reconnect {} time, next reconnect time {} seconds later",attempts, duration);
        ControlFlow::Continue(Duration::from_secs(duration))
    }
}

fn prompt(message: &str) -> Result<String> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    stdout.write_all(message.as_bytes())?;
    stdout.flush()?;

    let stdin = io::stdin();
    let mut stdin = stdin.lock();

    let mut line = String::new();
    stdin.read_line(&mut line)?;
    Ok(line)
}

async fn async_main() -> Result<()> {
    SimpleLogger::new()
        .with_level(log::LevelFilter::Debug)
        .init()
        .unwrap();

    // your api id
    let api_id = 1;
    // your api hash
    let api_hash: &str = "1";

    let reconnection_policy: Box<dyn ReconnectionPolicy> = Box::new(FixedReconnect {
        attempts: usize::MAX,
        delay: Duration::from_secs(5),
    });
    let init_params = InitParams {
        // app_version: "Telegram Desktop 5.6.1 x64".to_string(),
        lang_code: "en".to_string(),
        system_lang_code: "en".to_string(),
        // system_version: "Windows 11 x64".to_string(),
        // device_model: "Telegram Desktop".to_string(),
        // reconnection_policy: Box::leak(reconnection_policy),
        reconnection_policy: &AutoReconnectPolicy,
        // catch_up: true,
        ..Default::default()
    };
    info!("Connecting to Telegram...");
    let client = Client::connect(Config {
        params: init_params,
        api_id,
        api_hash: String::from(api_hash),
        session: Session::load_file_or_create(SESSION_FILE)?,
    })
        .await?;

    info!("Connected!");

    // If we can't save the session, sign out once we're done.
    let mut sign_out = false;

    if !client.is_authorized().await? {
        info!("Signing in...");
        let phone = prompt("Enter your phone number (international format),eg:+86 130 1234 5678: ")?;
        let token = client.request_login_code(&phone).await?;
        let code = prompt("Enter the code you received: ")?;
        let signed_in = client.sign_in(&token, &code).await;
        match signed_in {
            Err(SignInError::PasswordRequired(password_token)) => {
                // Note: this `prompt` method will echo the password in the console.
                //       Real code might want to use a better way to handle this.
                let hint = password_token.hint().unwrap_or("None");
                let prompt_message = format!("Enter the password (hint {}): ", &hint);
                let password = prompt(prompt_message.as_str())?;

                client
                    .check_password(password_token, password.trim())
                    .await?;
            }
            Ok(_) => (),
            Err(e) => panic!("{}", e),
        };
        info!("Signed!");
        match client.session().save_to_file(SESSION_FILE) {
            Ok(_) => {}
            Err(e) => {
                error!("NOTE: failed to save the session, will sign out when done: {e}");
                sign_out = true;
            }
        }
    }

    if sign_out {
        drop(client.sign_out_disconnect().await);
    }

    // 使用信号等待，保持程序运行，直到收到中断信号
    // signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
    // 监听 Ctrl+C 信号
    // if signal::ctrl_c().await.is_ok() {
    //     warn!("Exiting...");
    //     break;
    // }

    info!("Waiting for messages...");

    loop {
        info!("Loop ...");
        let exit = pin!(async { tokio::signal::ctrl_c().await });
        let upd = pin!(async { client.next_update().await });

        let update = match select(exit, upd).await {
            Either::Left(_) => {
                warn!("Exiting...");
                break;
            }
            Either::Right((u, _)) => u?,
        };

        let handle = client.clone();
        task::spawn(async move {
            if let Err(e) = handle_update(handle, update).await {
                error!("Error handling updates!: {e}")
            }
        });
    }

    info!("Shutting down...");

    Ok(())
}

async fn handle_update(client: Client, update: Update) -> Result<()> {
    match update {
        Update::NewMessage(message) if !message.outgoing() => {
            let chat = message.chat();

            info!("Received message from:");
            info!("pack type:{}", chat.pack().ty);
            info!("chat name:{}",chat.name());
            info!("message:{}", message.text());

            // 私人聊天自动回复
            if chat.pack().ty == PackedType::User {
                client.send_message(&chat, "I'm currently unavailable. This is an auto-reply message!").await?;
                info!("Reply message successful!")
            }
        }
        _ => {}
    }

    Ok(())
}

fn main() -> Result<()> {
    runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async_main())
}
