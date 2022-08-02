use anyhow::Result;
use chrono::{DateTime, Utc};
use reqwest::header::ACCEPT;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};

const APPLICATION_JSON: &str = "application/json";

#[derive(Debug, Clone)]
pub struct MailHog {
    client: reqwest::Client,
    base_url: String,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ListMessagesParams {
    start: Option<i64>,
    limit: Option<i64>,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SearchKind {
    #[serde(rename = "from")]
    From,
    #[serde(rename = "to")]
    To,
    #[serde(rename = "containing")]
    Containing,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SearchParams {
    kind: SearchKind,
    query: String,
    start: Option<i64>,
    limit: Option<i64>,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EmailAddr {
    #[serde(rename = "Mailbox")]
    mailbox: String,
    #[serde(rename = "Domain")]
    domain: String,
    #[serde(rename = "Params")]
    params: String,
    #[serde(rename = "Relays")]
    relays: Option<String>,
}

impl Display for EmailAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}", self.mailbox, self.domain)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MessageContent {
    #[serde(rename = "Headers")]
    headers: HashMap<String, Vec<String>>,
    #[serde(rename = "Body")]
    body: String,
    #[serde(rename = "Size")]
    size: usize,
    #[serde(rename = "MIME")]
    mime: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Message {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "From")]
    from: EmailAddr,
    #[serde(rename = "To")]
    to: Vec<EmailAddr>,
    #[serde(rename = "Content")]
    content: MessageContent,
    #[serde(rename = "Created")]
    created: DateTime<Utc>,
}

impl PartialOrd<Self> for Message {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.created.partial_cmp(&other.created)
    }
}

impl Ord for Message {
    fn cmp(&self, other: &Self) -> Ordering {
        self.created.cmp(&other.created)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MessageList {
    total: i64,
    start: i64,
    count: i64,
    #[serde(default)]
    items: Vec<Message>,
}

impl MailHog {
    pub fn new(base_url: String) -> MailHog {
        MailHog {
            client: reqwest::Client::new(),
            base_url,
        }
    }

    pub async fn list_messages(&self, params: ListMessagesParams) -> Result<MessageList> {
        Ok(self
            .client
            .execute(
                self.client
                    .get(format!("{}/api/v2/messages", self.base_url))
                    .query(&params)
                    .header(ACCEPT, APPLICATION_JSON)
                    .build()?,
            )
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    pub async fn search(&self, params: SearchParams) -> Result<MessageList> {
        Ok(self
            .client
            .execute(
                self.client
                    .get(format!("{}/api/v2/search", self.base_url))
                    .query(&params)
                    .header(ACCEPT, APPLICATION_JSON)
                    .build()?,
            )
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use crate::{ListMessagesParams, MailHog, SearchKind, SearchParams};
    use chrono::Utc;
    use lettre::transport::smtp::client::Tls;
    use lettre::{Message, SmtpTransport, Transport};
    use rand::distributions::Alphanumeric;
    use rand::{thread_rng, Rng};
    use testcontainers::clients::Cli;
    use testcontainers::images::generic::GenericImage;
    use testcontainers::Container;

    const SMTP_PORT: u16 = 1025;
    const HTTP_PORT: u16 = 8025;

    struct TestEnv<'a> {
        mh: MailHog,
        mailer: SmtpTransport,
        _container: Container<'a, GenericImage>,
    }

    struct MsgDetails {
        to: String,
        from: String,
        subject: String,
        body: String,
    }

    #[derive(Debug, Default, Clone)]
    struct MakeMessagesParams {
        to: Option<String>,
        from: Option<String>,
        subject: Option<String>,
        body: Option<String>,
    }

    fn make_rand_str(n: usize) -> String {
        thread_rng()
            .sample_iter(&Alphanumeric)
            .take(n)
            .map(char::from)
            .collect()
    }

    fn make_rand_email_addr(domain: Option<String>) -> String {
        format!(
            "{}@{}",
            make_rand_str(10),
            domain.unwrap_or_else(|| format!("{}.com", make_rand_str(10)))
        )
    }

    fn make_rand_messages(n: usize, params: MakeMessagesParams) -> Vec<MsgDetails> {
        (0..n)
            .map(|_| MsgDetails {
                to: params
                    .to
                    .clone()
                    .unwrap_or_else(|| make_rand_email_addr(None)),
                from: params
                    .from
                    .clone()
                    .unwrap_or_else(|| make_rand_email_addr(None)),
                subject: params.subject.clone().unwrap_or_else(|| make_rand_str(50)),
                body: params.body.clone().unwrap_or_else(|| make_rand_str(200)),
            })
            .collect()
    }

    fn make_rand_message(params: MakeMessagesParams) -> MsgDetails {
        make_rand_messages(1, params).swap_remove(0)
    }

    fn setup(cli: &Cli) -> TestEnv {
        let container = cli.run(
            GenericImage::new("mailhog/mailhog", "v1.0.1")
                .with_exposed_port(SMTP_PORT)
                .with_exposed_port(HTTP_PORT),
        );
        let smtp_port = container.get_host_port_ipv4(SMTP_PORT);
        let mailer = SmtpTransport::relay("localhost")
            .unwrap()
            .port(smtp_port)
            .tls(Tls::None)
            .build();

        let http_port = container.get_host_port_ipv4(HTTP_PORT);
        println!("mailhog http port: {}", http_port);
        TestEnv {
            _container: container,
            mh: MailHog::new(format!("http://localhost:{}", http_port)),
            mailer,
        }
    }

    fn normalize_body(b: impl AsRef<str>) -> String {
        b.as_ref().replace("=\r\n", "")
    }

    #[tokio::test]
    async fn list_messages() {
        let cli = Cli::docker();
        let env = setup(&cli);
        let mh = env.mh;
        let mailer = env.mailer;

        let message_list = mh
            .list_messages(ListMessagesParams {
                start: None,
                limit: None,
            })
            .await
            .unwrap();

        assert_eq!(0, message_list.total);
        assert_eq!(0, message_list.count);
        assert_eq!(0, message_list.start);
        assert_eq!(0, message_list.items.len());

        let msg = make_rand_message(Default::default());
        let from = msg.from.as_str();
        let to = msg.to.as_str();
        let subject = msg.subject.as_str();
        let body = msg.body.as_str();
        mailer
            .send(
                &Message::builder()
                    .from(from.parse().unwrap())
                    .to(to.parse().unwrap())
                    .subject(subject)
                    .body(body.to_string())
                    .unwrap(),
            )
            .unwrap();

        let message_list = mh
            .list_messages(ListMessagesParams {
                start: None,
                limit: None,
            })
            .await
            .unwrap();
        assert_eq!(1, message_list.total);
        assert_eq!(1, message_list.count);
        assert_eq!(0, message_list.start);
        assert_eq!(1, message_list.items.len());
        let message = &message_list.items[0];
        assert_eq!(from, message.from.to_string());
        assert_eq!(1, message.to.len());
        assert_eq!(to, message.to[0].to_string());
        assert_eq!(body, normalize_body(message.content.body.as_str()));
        assert!(message.content.size > body.len());
        assert!(message.created < Utc::now());

        const SUBJECT: &str = "Subject";
        assert!(message.content.headers.get(SUBJECT).is_some());
        assert_eq!(vec![subject], message.content.headers[SUBJECT]);
    }

    #[tokio::test]
    async fn search_messages() {
        let cli = Cli::docker();
        let env = setup(&cli);
        let mh = env.mh;
        let mailer = env.mailer;

        let from = make_rand_email_addr(None);
        let to = make_rand_email_addr(None);
        let subject_part = make_rand_str(10);
        let subject = subject_part.clone() + " " + &make_rand_str(30);
        let body_part = make_rand_str(10);
        let body = body_part.clone() + " " + &make_rand_str(200);

        let params = vec![
            (
                MakeMessagesParams {
                    from: Some(from.to_string()),
                    ..Default::default()
                },
                SearchParams {
                    kind: SearchKind::From,
                    query: from.to_string(),
                    start: None,
                    limit: None,
                },
            ),
            (
                MakeMessagesParams {
                    to: Some(to.to_string()),
                    ..Default::default()
                },
                SearchParams {
                    kind: SearchKind::To,
                    query: to.to_string(),
                    start: None,
                    limit: None,
                },
            ),
            (
                MakeMessagesParams {
                    subject: Some(subject.to_string()),
                    ..Default::default()
                },
                SearchParams {
                    kind: SearchKind::Containing,
                    query: subject_part.to_string(),
                    start: None,
                    limit: None,
                },
            ),
            (
                MakeMessagesParams {
                    body: Some(body.to_string()),
                    ..Default::default()
                },
                SearchParams {
                    kind: SearchKind::Containing,
                    query: body_part.to_string(),
                    start: None,
                    limit: None,
                },
            ),
        ];

        let num_messages = thread_rng().gen_range(1..50);

        for (make_messages_params, search_params) in params {
            let outbox = make_rand_messages(num_messages, make_messages_params);
            for m in &outbox {
                mailer
                    .send(
                        &Message::builder()
                            .from(m.from.parse().unwrap())
                            .to(m.to.parse().unwrap())
                            .subject(&m.subject)
                            .body(m.body.to_string())
                            .unwrap(),
                    )
                    .unwrap();
            }

            let mut message_list = mh.search(search_params).await.unwrap();
            message_list.items.sort();

            assert_eq!(num_messages, message_list.total as usize);
            assert_eq!(num_messages, message_list.count as usize);
            assert_eq!(0, message_list.start);
            assert_eq!(num_messages, message_list.items.len());

            for (idx, message) in message_list.items.iter().enumerate() {
                let from = outbox[idx].from.as_str();
                let to = outbox[idx].to.as_str();
                let subject = outbox[idx].subject.as_str();
                let body = outbox[idx].body.as_str();

                assert_eq!(from, message.from.to_string());
                assert_eq!(1, message.to.len());
                assert_eq!(to, message.to[0].to_string());
                assert_eq!(body, normalize_body(message.content.body.as_str()));
                assert!(message.content.size > body.len());
                assert!(message.created < Utc::now());

                const SUBJECT: &str = "Subject";
                assert!(message.content.headers.get(SUBJECT).is_some());
                assert_eq!(vec![subject], message.content.headers[SUBJECT]);
            }
        }
    }
}
