use crate::config::SmtpConfig;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use tokio::{
    sync::Mutex,
    time::{interval, Interval, MissedTickBehavior},
};
use tracing::{debug, log::error};

/// Type alias for the SMTP transport.
pub type SmtpTransport = AsyncSmtpTransport<Tokio1Executor>;

/// SMTP utilities.
pub struct Smtp {
    /// SMTP configuration.
    pub config: SmtpConfig,
    /// The SMTP transport.
    transport: SmtpTransport,
    /// The timestamp when the last email was attempted to be sent via SMTP.
    throttle_interval: Mutex<Interval>,
}

impl Smtp {
    /// Creates a new `Smtp` utilities instance.
    pub fn new(transport: SmtpTransport, config: SmtpConfig) -> Self {
        let mut throttle_interval = interval(config.throttle_delay);
        throttle_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        Self {
            transport,
            config,
            throttle_interval: Mutex::new(throttle_interval),
        }
    }

    /// Sends the specified email message using the SMTP transport.
    pub async fn send(&self, message: Message) -> anyhow::Result<()> {
        // Try to send email respecting the throttle delay.
        let mut interval = self.throttle_interval.lock().await;
        interval.tick().await;

        let smtp_response = self.transport.send(message).await;
        interval.reset();

        let smtp_response = smtp_response?;
        if smtp_response.is_positive() {
            debug!(
                "SMTP server succeeded with {}: {:?}",
                smtp_response.code(),
                smtp_response.first_line()
            );
        } else {
            error!(
                "SMTP server failed with {}: {:?}",
                smtp_response.code(),
                smtp_response.first_line()
            );
        }

        Ok(())
    }
}

#[cfg(test)]
pub mod tests {
    use crate::{
        config::SmtpConfig,
        tests::{mock_smtp, mock_smtp_config},
    };
    use anyhow::bail;
    use futures::future::join_all;
    use lettre::Message;
    use regex::bytes::Regex;
    use std::{
        collections::HashSet,
        io::{Read, Write},
        net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream},
        sync::{Arc, Mutex},
        time::Duration,
    };
    use test_log::test;
    use time::OffsetDateTime;
    use tracing::{error, info};
    use wg::WaitGroup;

    #[test(tokio::test)]
    async fn respects_throttle_delay() -> anyhow::Result<()> {
        let smtp_server = MockSmtpServer::new("smtp.retrack.dev");
        smtp_server.start();

        let smtp = mock_smtp(SmtpConfig {
            throttle_delay: Duration::from_millis(3200),
            ..mock_smtp_config(smtp_server.host.to_string(), smtp_server.port)
        });

        let messages = vec![
            Message::builder()
                .from("dev@retrack.dev".parse()?)
                .subject("subject-1")
                .to("to-1@retrack.dev".parse()?)
                .date(OffsetDateTime::from_unix_timestamp(946720700)?.into())
                .body("text-1".to_string())?,
            Message::builder()
                .from("dev@retrack.dev".parse()?)
                .subject("subject-2")
                .to("to-2@retrack.dev".parse()?)
                .date(OffsetDateTime::from_unix_timestamp(946720800)?.into())
                .body("text-2".to_string())?,
            Message::builder()
                .from("dev@retrack.dev".parse()?)
                .subject("subject-3")
                .to("to-3@retrack.dev".parse()?)
                .date(OffsetDateTime::from_unix_timestamp(946720900)?.into())
                .body("text-3".to_string())?,
        ];

        join_all(messages.into_iter().map(|message| smtp.send(message))).await;

        let mails = smtp_server.mails();
        assert_eq!(mails.len(), 3);

        let delay_between_mails_one: Duration =
            (mails[1].timestamp - mails[0].timestamp).try_into()?;
        info!(
            "Delay #1 {}.",
            humantime::format_duration(delay_between_mails_one)
        );

        let delay_between_mails_two: Duration =
            (mails[2].timestamp - mails[1].timestamp).try_into()?;
        info!(
            "Delay #2 {}.",
            humantime::format_duration(delay_between_mails_two)
        );
        assert!(delay_between_mails_one >= Duration::from_secs(3));
        assert!(delay_between_mails_one >= Duration::from_secs(3));

        Ok(())
    }

    /// A stripped down version of `maik` mock SMTP server library:
    /// Author: Coco Liliace
    /// Source: https://sr.ht/~liliace/maik/
    /// License: MPL-2.0
    const OK: &[u8] = b"250 OK\r\n";
    const AUTH_SUCCESS: &[u8] = b"235 2.7.0 Authentication successful\r\n";
    const AUTH_UNSUPPORTED_METHOD: &[u8] = b"504 5.5.4 Authentication method not supported\r\n";
    const BAD_MAILBOX_SYNTAX: &[u8] = b"553 Mailbox syntax incorrect\r\n";
    const NO_MAIL_TRANSACTION: &[u8] = b"503 No mail transaction in progress\r\n";

    /// A mock SMTP server.
    #[derive(Clone)]
    pub struct MockSmtpServer {
        listener: Arc<TcpListener>,
        pub host: IpAddr,
        pub port: u16,
        domain: String,
        mails: Arc<Mutex<Vec<Mail>>>,
        wg: WaitGroup,
        re_sep: Regex,
        re_mail_user: Regex,
        re_body: Regex,
    }

    impl MockSmtpServer {
        /// Creates a MockServer with the given domain.
        /// The domain is only used for the ready message and EHLO/HELO replies.
        pub fn new(domain: &str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind tcp listener");
            let addr = listener.local_addr().unwrap();
            let host = addr.ip();
            Self {
                listener: Arc::new(listener),
                host,
                port: addr.port(),
                domain: String::from(domain),
                mails: Arc::new(Mutex::new(Vec::new())),
                wg: WaitGroup::new(),
                re_sep: Regex::new(r"( |\r\n)").unwrap(),
                re_mail_user: Regex::new(r".*<(.*?)>").unwrap(),
                re_body: Regex::new(r"(?s)(.*?)(\r\n\.\r\n|\z)").unwrap(),
            }
        }

        /// Asserts the given assertion against the server.
        pub fn mails(self) -> Vec<Mail> {
            self.wg.wait();
            self.mails.to_owned().lock().unwrap().to_vec()
        }

        /// Starts the server.
        pub fn start(&self) {
            let server = Arc::new(Mutex::new(self.clone()));
            let server = Arc::clone(&server);
            std::thread::spawn(move || {
                let mut server = server.lock().unwrap();
                while let Ok((stream, socket)) = server.listener.accept() {
                    server.wg.add(1);
                    server.handle_client(socket, stream);
                    server.wg.done();
                }
            });
        }

        fn ready_message(&self) -> Vec<u8> {
            format!("220 {} Service ready\r\n", self.domain)
                .as_bytes()
                .to_vec()
        }

        fn handle_client(&mut self, socket: SocketAddr, mut stream: TcpStream) {
            if let Err(e) = stream.write_all(&self.ready_message()) {
                error!("Failed to send READY to {socket}: {e}");
            }

            let mut buffer = vec![0; 2048];
            let mut client_state = ClientState::new();

            while let Ok(size) = stream.read(&mut buffer) {
                let data = &buffer[..size];
                if let Ok(data_str) = std::str::from_utf8(data) {
                    info!("{socket} sent: {data_str}");
                } else {
                    error!("{socket} sent invalid UTF-8 data");
                }

                match self.handle_stream(stream, data, &mut client_state) {
                    Ok(new_stream) => stream = new_stream,
                    Err(e) => {
                        error!("Failed to send response to {socket}: {e}");
                        break;
                    }
                }
            }
            info!("{socket} closed");
        }

        fn handle_stream(
            &mut self,
            mut stream: TcpStream,
            data: &[u8],
            client_state: &mut ClientState,
        ) -> anyhow::Result<TcpStream> {
            if data.is_empty() {
                // same client reconnecting after QUIT
                stream.write_all(&self.ready_message())?;
            }

            if client_state.mail_transaction.is_receiving_data {
                let captures = self.re_body.captures(data).unwrap();
                let body = captures.get(1).unwrap().as_bytes();
                let mut i = 0;
                while i < body.len() {
                    if i + 3 < body.len() && body[i..i + 4] == *b"\r\n.." {
                        client_state.mail_transaction.body.extend(b"\r\n.");
                        i += 4;
                    } else {
                        client_state.mail_transaction.body.push(body[i]);
                        i += 1;
                    }
                }
                if captures.get(2).unwrap().as_bytes() == b"\r\n.\r\n" {
                    self.mails.lock().unwrap().push(Mail {
                        content: client_state.mail_transaction.body.to_owned(),
                        timestamp: OffsetDateTime::now_utc(),
                    });
                    client_state.mail_transaction.reset();
                }
                stream.write_all(OK)?;
                return Ok(stream);
            }

            if let AuthState::Plain = client_state.auth_state {
                client_state.auth_state = AuthState::Completed;
                stream.write_all(AUTH_SUCCESS)?;
                return Ok(stream);
            }

            let (command, arg) = {
                // Trim the \r\n and split data into command and argument.
                let mut parts = self.re_sep.splitn(&data[..data.len() - 2], 2);
                let Some(command) = parts.next().map(|command| command.to_ascii_lowercase()) else {
                    bail!("Cannot parse command.");
                };
                (command, parts.next().unwrap_or_default().to_vec())
            };
            self.handle_command(stream, command, arg, client_state)
        }

        fn handle_command(
            &mut self,
            mut stream: TcpStream,
            command: Vec<u8>,
            arg: Vec<u8>,
            client_state: &mut ClientState,
        ) -> anyhow::Result<TcpStream> {
            if command == b"ehlo" {
                client_state.reset();
                client_state.has_ehloed = true;
                stream.write_all(
                    format!(
                        "250-{}\r\n\
                    250 AUTH PLAIN\r\n",
                        self.domain
                    )
                    .as_bytes(),
                )?;
            } else if command == b"helo" {
                client_state.reset();
                client_state.has_ehloed = true;
                stream.write_all(format!("250 {}\r\n", &self.domain).as_bytes())?;
            } else if command == b"quit" {
                stream.write_all(b"221 OK\r\n")?;
                stream.shutdown(Shutdown::Both)?;
            } else if command == b"noop" {
                stream.write_all(OK)?;
            } else if command == b"auth" {
                if client_state.auth_state == AuthState::Completed {
                    stream.write_all(b"503 Already authenticated\r\n")?;
                } else if client_state.mail_transaction.sender.is_some() {
                    stream.write_all(b"503 Cannot authenticate during a mail transaction\r\n")?;
                } else {
                    let mut parts = arg.splitn(2, |&c| c == b' ');
                    let method = parts.next().unwrap_or_default().to_ascii_lowercase();
                    let answer = parts.next().unwrap_or_default();
                    if answer.is_empty() {
                        if (match method.as_slice() {
                            b"plain" => {
                                client_state.auth_state = AuthState::Plain;
                                Ok(())
                            }
                            _ => Err(stream.write_all(AUTH_UNSUPPORTED_METHOD)?),
                        })
                        .is_ok()
                        {
                            stream.write_all(b"334 \r\n")?; // intentional trailing space
                        }
                    } else if answer == b"*" {
                        stream.write_all(b"501 Authentication cancelled by client\r\n")?;
                    } else {
                        client_state.auth_state = AuthState::Completed;

                        // https://github.com/lettre/lettre/issues/970
                        // client_state.has_ehloed = false;
                        stream.write_all(AUTH_SUCCESS)?;
                    }
                }
            } else if command == b"mail" {
                if !client_state.has_ehloed {
                    stream.write_all(b"503 Session has not been opened with EHLO/HELO\r\n")?;
                } else if client_state.mail_transaction.sender.is_some() {
                    stream.write_all(b"503 A mail transaction is already in progress\r\n")?;
                } else if client_state.auth_state != AuthState::Completed {
                    stream.write_all(b"530 5.7.0 Authentication required\r\n")?;
                } else {
                    // arg is in format of FROM:optional name <email>
                    match self
                        .re_mail_user
                        .captures(&arg[5..])
                        .map(|c| c.get(1))
                        .map(|c| c.unwrap().as_bytes())
                    {
                        Some(address) => {
                            client_state.mail_transaction.sender = Some(address.to_vec());
                            stream.write_all(OK)?;
                        }
                        None => stream.write_all(BAD_MAILBOX_SYNTAX)?,
                    }
                }
            } else if command == b"data" {
                if client_state.mail_transaction.sender.is_none() {
                    stream.write_all(NO_MAIL_TRANSACTION)?;
                } else if client_state.mail_transaction.recipients.is_empty() {
                    stream.write_all(b"503 No recipients\r\n")?;
                } else {
                    client_state.mail_transaction.is_receiving_data = true;
                    stream.write_all(b"354 Start mail input; end with <CRLF>.<CRLF>\r\n")?;
                }
            } else if command == b"rset" {
                client_state.reset();
                stream.write_all(OK)?;
            } else if command == b"rcpt" {
                if client_state.mail_transaction.sender.is_none() {
                    stream.write_all(NO_MAIL_TRANSACTION)?;
                } else {
                    // arg is in format of TO:<email>
                    match self
                        .re_mail_user
                        .captures(&arg[3..])
                        .map(|c| c.get(1))
                        .map(|c| c.unwrap().as_bytes())
                    {
                        Some(recipient) => {
                            client_state
                                .mail_transaction
                                .recipients
                                .insert(recipient.to_vec());
                            stream.write_all(OK)?;
                        }
                        None => stream.write_all(BAD_MAILBOX_SYNTAX)?,
                    }
                }
            } else if command == b"vrfy" {
                stream.write_all(b"252\r\n")?;
            } else {
                stream.write_all(b"500 Unrecognized command\r\n")?;
            }
            Ok(stream)
        }
    }

    #[derive(Clone)]
    struct ClientState {
        pub mail_transaction: MailTransaction,
        pub has_ehloed: bool,
        pub auth_state: AuthState,
    }

    #[derive(Clone)]
    struct MailTransaction {
        pub sender: Option<Vec<u8>>,
        pub recipients: HashSet<Vec<u8>>,
        pub body: Vec<u8>,
        pub is_receiving_data: bool,
    }

    #[derive(Clone, PartialEq)]
    enum AuthState {
        NotStarted,
        Plain,
        Completed,
    }

    impl ClientState {
        pub fn new() -> Self {
            Self {
                mail_transaction: MailTransaction::new(),
                has_ehloed: false,
                auth_state: AuthState::NotStarted,
            }
        }

        pub fn reset(&mut self) {
            self.mail_transaction.reset();
            self.auth_state = AuthState::NotStarted;
        }
    }

    impl MailTransaction {
        fn new() -> Self {
            Self {
                sender: None,
                recipients: HashSet::new(),
                body: Vec::new(),
                is_receiving_data: false,
            }
        }
        pub fn reset(&mut self) {
            self.sender = None;
            self.recipients.clear();
            self.body.clear();
            self.is_receiving_data = false;
        }
    }

    #[derive(Clone)]
    pub struct Mail {
        pub content: Vec<u8>,
        pub timestamp: OffsetDateTime,
    }
}
