use crate::config::AuthMethod;
use anyhow::{anyhow, Context, Result};
use async_imap::types::Seq;
use async_native_tls::{TlsConnector, TlsStream};
use async_std::{
    net::TcpStream,
    sync::{Mutex, MutexGuard},
    task,
};
use futures::StreamExt;
use std::{collections::VecDeque, vec};

pub type ImapClient = async_imap::Client<TlsStream<TcpStream>>;
pub type MailboxName = async_imap::types::Name;
pub type ImapSession = async_imap::Session<TlsStream<TcpStream>>;
pub type ImapResult<T> = async_imap::error::Result<T>;
pub type ImapIdleHandle = async_imap::extensions::idle::Handle<TlsStream<TcpStream>>;

struct SessionHandle<'a> {
    session: MutexGuard<'a, Option<ImapSession>>,
}
impl<'a> SessionHandle<'a> {
    pub fn new(session: MutexGuard<'a, Option<ImapSession>>) -> Self {
        Self { session }
    }
    pub fn get(&mut self) -> &mut ImapSession {
        self.session.as_mut().unwrap()
    }
    pub fn take(&mut self) -> Option<ImapSession> {
        self.session.take()
    }
    pub fn replace(&mut self, session: Option<ImapSession>) -> Option<ImapSession> {
        let prev = self.session.take();
        *self.session = session;
        prev
    }
}

pub trait MailPath {
    fn path(&self) -> String;
}
impl MailPath for MailboxName {
    fn path(&self) -> String {
        match self.delimiter() {
            Some(delimiter) => self.name().to_owned().replace(delimiter, "/"),
            None => self.name().to_owned(),
        }
    }
}

pub struct ImapConnection {
    server: String,
    port: u16,
    auth: AuthMethod,
    session: Mutex<Option<ImapSession>>,
}
impl ImapConnection {
    pub fn new(server: String, port: u16, auth: AuthMethod) -> Self {
        Self {
            server,
            port,
            auth,
            session: Mutex::new(None),
        }
    }
    fn client(&self) -> Result<ImapClient> {
        let tls = TlsConnector::new();
        let client = task::block_on(async_imap::connect(
            (self.server.as_str(), self.port),
            self.server.clone(),
            tls,
        ))
        .context("Failed to connect to IMAP server.")?;
        Ok(client)
    }
    async fn session(&self) -> Result<SessionHandle<'_>> {
        if self.session.lock().await.is_none() {
            let client = self.client()?;
            let session = match self.auth.clone() {
                AuthMethod::Login { user, password } => {
                    task::block_on(client.login(user, password))
                }
                _ => {
                    //TODO: implement
                    unimplemented!();
                }
            }
            .map_err(|(e, _)| e)
            .context("Failed to authenticate with the IMAP server.")?;
            self.session.lock().await.replace(session);
        }

        Ok(SessionHandle::new(self.session.lock().await))
    }
    async fn take_session(&mut self) -> Result<ImapSession> {
        self.session()
            .await?
            .take()
            .ok_or_else(|| anyhow!("Failed to take IMAP session"))
    }
    pub async fn run<F, R>(&self, runfn: F) -> Result<R>
    where
        F: Fn(&mut ImapSession) -> ImapResult<R>,
    {
        let mut retry = 0;
        loop {
            let mut session_handle = self.session().await?;
            let run_result = runfn(session_handle.get());
            match run_result {
                Ok(result) => return Ok(result),
                Err(async_imap::error::Error::ConnectionLost) => {
                    // Throw away currently cached session
                    let _ = session_handle.replace(None);
                }
                Err(e) => {
                    retry += 1;
                    if retry >= 3 {
                        Err(e).context("IMAP request failed")? // other errors are directly returned
                    }
                }
            };
        }
    }

    async fn recursive_mailbox_list(&self) -> Result<Vec<async_imap::types::Name>> {
        let mut session_handle = self.session().await?;
        let result: Vec<ImapResult<_>> = session_handle
            .get()
            .list(None, Some("*"))
            .await
            .context("Failed to acquire recursive list of mailboxes")?
            .collect::<_>()
            .await;
        // fail if any single item in the stream failed
        let result: ImapResult<Vec<_>> = result.into_iter().collect();
        Ok(result?)
    }

    async fn fetch_mail(&self, message_id: String) -> Result<async_imap::types::Fetch> {
        let mut session_borrow = self.session().await?;
        let session_borrow = session_borrow.get();
        let mut message_stream = session_borrow.fetch(&message_id, "RFC822").await?;
        if let Some(message) = message_stream.next().await {
            Ok(message?)
        } else {
            Err(anyhow!("Failed to fetch message: {}", message_id))
        }
    }

    pub async fn delete_mails(&self, message_ids: &[Seq]) -> Result<()> {
        let id_list: String = message_ids.iter().fold("".to_owned(), |a, b| {
            if a.is_empty() {
                b.to_string()
            } else {
                format!("{},{}", a, b)
            }
        });

        // Add \Delete flags to messages
        let flag_result: Vec<ImapResult<_>> = self
            .session()
            .await?
            .get()
            .store(id_list, "+FLAGS (\\Deleted)")
            .await
            .context("Failed to mark mails with Deleted flag")?
            .collect()
            .await;
        let flag_result: ImapResult<Vec<_>> = flag_result.into_iter().collect();

        let expunge_result: Vec<ImapResult<_>> = self
            .session()
            .await?
            .get()
            .expunge()
            .await
            .context("Failed to delete messages marked for deletion")?
            .collect()
            .await;
        let expunge_result: ImapResult<Vec<_>> = expunge_result.into_iter().collect();

        // try to expunge the messages that were correctly marked, before throwing
        flag_result?;
        expunge_result?;
        Ok(())
    }

    pub fn iter_mailboxes_recursive(
        &self,
        path_filter: Option<&str>,
    ) -> Result<vec::IntoIter<MailboxName>> {
        // get a (linearized) list of the folder structure
        let mut mailboxes = task::block_on(self.recursive_mailbox_list())?;
        if let Some(filter) = path_filter {
            mailboxes.retain(|mailbox| {
                // Match the given filter against the "/"-delimited absolute path
                mailbox.path().starts_with(filter)
            });
        }
        Ok(mailboxes.into_iter())
    }

    pub async fn iter_unseen(&self, mailbox: &MailboxName) -> Result<UnseenMailIterator<'_>> {
        // select new mailbox and get a list of new/unseen messages
        let unread_mails: Vec<_> = self
            .run(|sess| {
                task::block_on(sess.select(mailbox.name()))?;
                task::block_on(sess.search("UNDELETED UNSEEN"))
            })
            .await?
            .into_iter()
            .collect();
        Ok(UnseenMailIterator {
            con: self,
            unread_mails: VecDeque::from(unread_mails),
        })
    }

    pub async fn idle(&mut self) -> Result<ImapIdleHandle> {
        let mut idle_handle = self.take_session().await?.idle();
        task::block_on(idle_handle.init())
            .context("Failed to initialize IDLE session with IMAP server")?;
        Ok(idle_handle)
    }
}
impl Drop for ImapConnection {
    fn drop(&mut self) {
        if let Ok(session) = &mut task::block_on(self.take_session()) {
            let _ = task::block_on(session.logout());
        }
    }
}

pub struct UnseenMailIterator<'a> {
    con: &'a ImapConnection,
    unread_mails: VecDeque<Seq>,
}
impl Iterator for UnseenMailIterator<'_> {
    type Item = Result<(Seq, Vec<u8>)>;

    fn next(&mut self) -> Option<Self::Item> {
        self.unread_mails.pop_front().map(|message_id| {
            match task::block_on(self.con.fetch_mail(message_id.to_string())) {
                Ok(fetch_result) => fetch_result
                    .body()
                    .map(|body| (message_id, body.to_vec()))
                    .ok_or_else(|| anyhow!("Failed to fetch message: {}", message_id)),
                Err(err) => Err(err),
            }
        })
    }
}
