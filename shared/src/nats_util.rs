use async_nats;
use clap::Parser;
use std::fs;
use std::io;

/// Arguments for the connection the the NATS server that each extractor and
/// tool needs.
#[derive(Parser, Debug, Clone, Default)]
pub struct NatsArgs {
    /// The NATS server address the extractor/tool should connect and subscribe to.
    #[arg(short = 'a', long = "nats-address", default_value = "127.0.0.1:4222")]
    pub address: String,

    /// The NATS username the extractor/tool should try to authentificate to the NATS server with.
    #[arg(short = 'u', long = "nats-username", default_value = None)]
    pub username: Option<String>,

    /// The NATS password the extractor/tool should try to authentificate to the NATS server with.
    #[arg(short = 'p', long = "nats-password", default_value = None)]
    pub password: Option<String>,

    /// A path to a file containing a password the extractor/tool should try to authentificate to
    /// the NATS server with.
    #[arg(short = 'f', long = "nats-password-file", default_value = None)]
    pub password_file: Option<String>,
}

/// Populates ConnectOptions with a username and password, if the passed
/// arguments contain one.
pub fn prepare_connection(args: &NatsArgs) -> Result<async_nats::ConnectOptions, io::Error> {
    match &args.username {
        Some(user) => {
            let mut pass: Option<String> = None;
            if let Some(password) = &args.password {
                log::debug!("Using supplied NATS user={} and password=***", user);
                pass = Some(password.to_string())
            } else if let Some(pw_file) = &args.password_file {
                let mut password = fs::read_to_string(pw_file)?;
                password = password.trim().to_string();
                log::info!(
                    "Using supplied NATS user={} with password from file {}",
                    user,
                    pw_file
                );
                pass = Some(password)
            }

            // TODO: do more than warn?
            if pass.is_none() {
                log::warn!(
                    "No NATS password supplied for connection to NATS server {} with user={}",
                    args.address,
                    user,
                );
            }

            log::info!(
                "Connecting to NATS-server {} with user={} and password=***",
                args.address,
                user
            );
            Ok(
                async_nats::ConnectOptions::new()
                    .user_and_password(user.to_string(), pass.unwrap()),
            )
        }
        None => {
            log::debug!(
                "Connecting to NATS-server at {} without authentification",
                args.address
            );
            Ok(async_nats::ConnectOptions::new())
        }
    }
}
