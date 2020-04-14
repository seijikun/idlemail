use crate::{config::AuthMethod, hub::Mail};
use anyhow::{anyhow, Context, Result};
use async_imap::types::Seq;
use async_native_tls::{TlsConnector, TlsStream};
use async_std::{net::TcpStream, prelude::*, task};
use std::{collections::VecDeque, iter::FromIterator, cell::{RefMut, RefCell}, borrow::BorrowMut};
pub trait MailPath {
    fn path(&self) -> String;
}
impl MailPath for async_imap::types::Name {
    fn path(&self) -> String {
        match self.delimiter() {
            Some(delimiter) => self.name().to_owned().replace(delimiter, "/"),
            None => self.name().to_owned(),
        }
    }
}

pub type ImapClient = async_imap::Client<TlsStream<TcpStream>>;
pub type ImapSession = async_imap::Session<TlsStream<TcpStream>>;
pub type ImapResult<T> = async_imap::error::Result<T>;
pub type ImapIdleHandle = async_imap::extensions::idle::Handle<TlsStream<TcpStream>>;
pub struct ImapConnection {
    server: String,
    port: u16,
    auth: AuthMethod,
    session: RefCell<Option<ImapSession>>,
}
impl ImapConnection {
    pub fn new(server: String, port: u16, auth: AuthMethod) -> Self {
        Self {
            server,
            port,
            auth,
            session: RefCell::new(None),
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
    fn session(&self) -> Result<RefMut<ImapSession>> {
        if self.session.borrow().is_none() {
            let client = self.client()?;
            let session = match self.auth.clone() {
                AuthMethod::Plain {
                    user: _,
                    password: _,
                } => {
                    //TODO: implement
                    unimplemented!();
                }
                AuthMethod::Login { user, password } => {
                    task::block_on(client.login(user, password))
                }
            }
            .map_err(|(e, _)| e)
            .context("Failed to authenticate with the IMAP server.")?;
            self.session.replace(Some(session));
        }

        Ok(RefMut::map(self.session.borrow_mut(), |s| s.as_mut().unwrap()))
    }
    fn take_session(&mut self) -> Result<ImapSession> {
        let _ = self.session()?;
        self.session
            .borrow_mut()
            .take()
            .ok_or_else(|| anyhow!("Failed to take IMAP session"))
    }
    pub fn run<F, R>(&self, runfn: F) -> Result<R>
    where
        F: Fn(&mut ImapSession) -> ImapResult<R>,
    {
        let mut retry = 0;
        loop {
            let run_result = runfn(self.session()?.borrow_mut());
            match run_result {
                Ok(result) => return Ok(result),
                Err(async_imap::error::Error::ConnectionLost) => {
                    // Throw away currently cached session
                    let _ = self.session.replace(None);
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
        let result = self
            .session()?
            .list(None, Some("*"))
            .await
            .context("Failed to acquire recursive list of mailboxes")?
            .collect::<ImapResult<_>>()
            .await
            .context("Failed to acquire recursive list of mailboxes")?;
        Ok(result)
    }

    async fn fetch_mail(&self, message_id: String) -> Result<async_imap::types::Fetch> {
        let mut session_borrow = self.session()?;
        let message_stream = session_borrow.borrow_mut().fetch(&message_id, "RFC822").await?;
        let mut messages: VecDeque<_> = message_stream.collect::<ImapResult<_>>().await?;
        messages
            .pop_front()
            .ok_or_else(|| anyhow!("Failed to fetch message: {}", message_id))
    }

    async fn delete_mails(&self, message_ids: &[Seq]) -> Result<()> {
        let id_list: String = message_ids.iter().fold("".to_owned(), |a, b| {
            if a.is_empty() {
                b.to_string()
            } else {
                format!("{},{}", a, b)
            }
        });

        // Add \Delete flags to messages
        let _updates: Vec<_> = self
            .session()?
            .store(id_list, "+FLAGS (\\Deleted)")
            .await?
            .collect::<ImapResult<_>>()
            .await?;
        // Expunge messages marked with \Delete
        let _upates: Vec<_> = self
            .session()?
            .expunge()
            .await?
            .collect::<ImapResult<_>>()
            .await?;
        Ok(())
    }

    pub fn iter_unseen_recursive(
        &self,
        filter: Option<&str>,
        delete: bool,
    ) -> Result<UnseenMailIterator> {
        // get a (linearized) list of the folder structure
        let mut mailboxes = task::block_on(self.recursive_mailbox_list())?;
        if let Some(filter) = filter {
            mailboxes = mailboxes
                .into_iter()
                .filter(|mailbox| {
                    // Match the given filter against the "/"-delimited absolute path
                    mailbox.path().starts_with(filter)
                })
                .collect();
        }
        Ok(UnseenMailIterator {
            con: self,
            delete,
            mailboxes,
            mailbox_idx: 0,
            unread_mails: Vec::new(),
            unread_mail_idx: 0,
        })
    }

    pub fn idle(&mut self) -> Result<ImapIdleHandle> {
        let mut idle_handle = self.take_session()?.idle();
        task::block_on(idle_handle.init())
            .context("Failed to initialize IDLE session with IMAP server")?;
        Ok(idle_handle)
    }
}
impl Drop for ImapConnection {
    fn drop(&mut self) {
        if let Ok(session) = &mut self.take_session() {
            let _ = task::block_on(session.logout());
        }
    }
}

pub struct UnseenMailIterator<'a> {
    con: &'a ImapConnection,
    delete: bool,
    mailboxes: Vec<async_imap::types::Name>,
    mailbox_idx: usize,
    unread_mails: Vec<Seq>,
    unread_mail_idx: usize,
}
impl<'a> Iterator for UnseenMailIterator<'a> {
    type Item = Result<(String, Mail)>;

    fn next(&mut self) -> Option<Self::Item> {
        // iterate through all mailboxes, searching for unread mails
        // if all mailboxes were tested, exit
        loop {
            if self.unread_mail_idx == self.unread_mails.len() {
                // first call, or no unseen mails remaining in the current mailbox

                // if we just finished iterating over all unread messages in a mailbox,
                // and configuration tells us to delete fetched messages, we batch-mark
                // all fetched messages in the currently (still) selected mailbox as
                // deleted
                if !self.unread_mails.is_empty() && self.delete {
                    if let Err(e) = task::block_on(self.con.delete_mails(&self.unread_mails)) {
                        return Some(Err(e));
                    }
                }

                // -> select next (if there is another one)
                if self.mailbox_idx == self.mailboxes.len() {
                    return None; // no new mailboxes to test
                }

                let mailbox = self.mailboxes.get(self.mailbox_idx)?;
                // select new mailbox and get a list of new/unseen messages
                self.unread_mails = Vec::from_iter(
                    self.con
                        .run(|sess| {
                            task::block_on(sess.select(mailbox.name()))?;
                            task::block_on(sess.search("UNDELETED UNSEEN"))
                        })
                        .unwrap()
                        .into_iter(),
                );
                self.unread_mail_idx = 0;
                self.mailbox_idx += 1;
            }

            // if there is an unread message in the currently selected mailbox...
            if let Some(message_id) = self.unread_mails.get(self.unread_mail_idx) {
                self.unread_mail_idx += 1;
                let path = self.mailboxes[self.mailbox_idx].path();

                // fetch and return it.
                return Some(
                    match task::block_on(self.con.fetch_mail(message_id.to_string())) {
                        Ok(fetch_result) => fetch_result
                            .body()
                            .map(|body| (path, Mail::from_rfc822(body.to_vec())))
                            .ok_or_else(|| anyhow!("Failed to fetch message: {}", message_id)),
                        Err(err) => Err(err),
                    },
                );
            }
        }
    }
}
