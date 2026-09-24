//! The decisions a file waits on before it joins the library.

use super::*;

impl LibraryService {
    /// Files waiting for a decision.
    pub fn pending_reviews(&self) -> Result<Vec<ImportReview>> {
        let profile_id = self.context.require_active_profile()?;
        self.ports
            .reviews
            .list_for_profile(profile_id, ReviewState::Pending)
    }

    /// The waiting decisions, with enough about each to make it.
    ///
    /// A row of the review queue names a file the listener has never seen: it
    /// was held back before it reached the library. What they need is where it
    /// is, why it is waiting, and — for a duplicate — what it is a duplicate
    /// *of*, which is a track they do know.
    pub fn review_cards(&self) -> Result<Vec<ReviewCard>> {
        let profile_id = self.context.require_active_profile()?;
        let pending = self
            .ports
            .reviews
            .list_for_profile(profile_id, ReviewState::Pending)?;

        let mut cards = Vec::with_capacity(pending.len());
        for review in pending {
            let path = self
                .ports
                .media_files
                .get(review.media_file_id)?
                .map(|file| file.path);

            let existing = match review.duplicate_media_file_id {
                Some(id) => self.ports.tracks.summary(profile_id, id)?,
                None => None,
            };

            cards.push(ReviewCard {
                id: review.id,
                reason: review.reason,
                path,
                existing,
            });
        }
        Ok(cards)
    }

    /// Applies the listener's decision about a file held back for review.
    pub fn resolve_review(
        &self,
        review_id: ImportReviewId,
        resolution: ReviewResolution,
    ) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let now = self.context.now();

        let review = self
            .ports
            .reviews
            .list_for_profile(profile_id, ReviewState::Pending)?
            .into_iter()
            .find(|entry| entry.id == review_id)
            .ok_or_else(|| CoreError::not_found("review entry", review_id))?;

        match resolution {
            // The new file is catalogued but never joins the library.
            ReviewResolution::KeepExisting => {}

            ReviewResolution::AddAnyway | ReviewResolution::EditMetadata => {
                self.add_to_library(profile_id, review.media_file_id, now)?;
            }

            ReviewResolution::RemoveExisting => {
                if let Some(existing) = review.duplicate_media_file_id {
                    self.ports.tracks.remove(profile_id, existing, now)?;
                }
                self.add_to_library(profile_id, review.media_file_id, now)?;
            }
        }

        self.ports
            .reviews
            .set_state(review_id, ReviewState::Resolved, now)?;
        self.context.events.publish(DomainEvent::LibraryChanged);
        Ok(())
    }

    /// True when this file already has an unresolved entry in the review queue.
    pub(super) fn awaiting_decision(
        &self,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
    ) -> Result<bool> {
        Ok(self
            .ports
            .reviews
            .list_for_profile(profile_id, ReviewState::Pending)?
            .iter()
            .any(|entry| entry.media_file_id == media_file_id))
    }

    /// Puts a file in front of the listener.
    pub(super) fn raise_review(
        &self,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
        duplicate_of: Option<MediaFileId>,
        reason: ReviewReason,
        now: Timestamp,
    ) -> Result<()> {
        // Rescanning the same unresolved problem must not stack up entries.
        let already_pending = self
            .ports
            .reviews
            .list_for_profile(profile_id, ReviewState::Pending)?
            .into_iter()
            .any(|entry| entry.media_file_id == media_file_id && entry.reason == reason);
        if already_pending {
            return Ok(());
        }

        let review = ImportReview {
            id: ImportReviewId::new(),
            profile_id,
            media_file_id,
            duplicate_media_file_id: duplicate_of,
            reason,
            state: ReviewState::Pending,
            created_at: now,
            resolved_at: None,
        };
        self.ports.reviews.save(&review)?;
        self.context.events.publish(DomainEvent::ReviewPending);
        Ok(())
    }
}
