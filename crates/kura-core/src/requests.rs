//! Requests: users ask, admins accept or reject. Accepted requests are queued as if the
//! admin had queued them.

use kura_domain::{Picked, Request, RequestState, SiteId};
use kura_store::NewRequest;

use crate::{Choices, Core, CoreError, Event, Result, Selection, auth::now};

impl Core {
    /// Stores a request after the same checks a download gets. Returns its id.
    pub async fn request(
        &self,
        site: SiteId,
        slug: &str,
        picked: &Picked,
        choices: &Choices,
        user_id: i64,
    ) -> Result<i64> {
        let source = self.site(site)?;
        self.check_choices(choices)?;
        if !picked.all && picked.seasons.is_empty() && picked.episodes.is_empty() {
            return Err(CoreError::Invalid("nothing was selected"));
        }
        let series = source.series(slug).await?;
        let id = self.store.create_request(&NewRequest {
            user_id,
            site,
            slug,
            series_title: &series.summary.title,
            picked,
            language: choices.language,
            hoster: choices.hoster.as_deref(),
            imdb_id: choices.imdb_id.as_deref(),
            now: now(),
        })?;
        let _ = self.events.send(Event::RequestsChanged);
        Ok(id)
    }

    /// All requests, or only `user`'s.
    pub fn requests(&self, user: Option<i64>) -> Result<Vec<Request>> {
        Ok(self.store.requests(user)?)
    }

    /// Queues what the request asks for and returns how many episodes were new. The download
    /// goes to the site's default library, else the main library.
    pub async fn accept_request(&self, id: i64, admin_id: i64) -> Result<usize> {
        let request = self
            .store
            .decide_request(id, RequestState::Accepted, now())?;
        let request = request.ok_or(CoreError::Invalid("this request was already decided"))?;
        let choices = Choices {
            language: request.language,
            hoster: request.hoster.clone(),
            library: None,
            imdb_id: request.imdb_id.clone(),
        };
        let selection = Selection::from_picked(&request.picked);
        let queued = self.enqueue(request.site, &request.slug, selection, &choices, admin_id);
        let queued = match queued.await {
            Ok(queued) => queued,
            Err(err) => {
                // Left open, so it can be accepted again once the cause is fixed.
                self.store.reopen_request(id)?;
                return Err(err);
            }
        };
        let _ = self.events.send(Event::RequestsChanged);
        Ok(queued)
    }

    /// Withdraws one of `user_id`'s own requests while it is still open.
    pub fn cancel_request(&self, id: i64, user_id: i64) -> Result<()> {
        if !self.store.cancel_request(id, user_id)? {
            return Err(CoreError::Invalid(
                "only your own open requests can be cancelled",
            ));
        }
        let _ = self.events.send(Event::RequestsChanged);
        Ok(())
    }

    /// Removes accepted and rejected requests from the list. Returns how many.
    pub fn clear_requests(&self) -> Result<usize> {
        let cleared = self.store.clear_decided_requests()?;
        let _ = self.events.send(Event::RequestsChanged);
        Ok(cleared)
    }

    pub fn reject_request(&self, id: i64) -> Result<()> {
        let decided = self
            .store
            .decide_request(id, RequestState::Rejected, now())?;
        decided.ok_or(CoreError::Invalid("this request was already decided"))?;
        let _ = self.events.send(Event::RequestsChanged);
        Ok(())
    }
}
