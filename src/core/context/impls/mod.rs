mod bg_game;
mod clients;
mod guilds;
mod links;
mod pp_stars;
mod shutdown;
mod twitch;

use crate::{Context, OsuTracking};

use twilight_http::error::Error as HttpError;
use twilight_model::{
    channel::{Message, Reaction},
    id::{ChannelId, MessageId, RoleId},
};

impl Context {
    /// Returns whether a message was sent by us
    pub fn is_own(&self, other: &Message) -> bool {
        self.cache
            .current_user()
            .map_or(false, |user| user.id == other.author.id)
    }

    pub fn get_role_assign(&self, reaction: &Reaction) -> Option<RoleId> {
        self.data
            .role_assigns
            .get(&(reaction.channel_id.0, reaction.message_id.0))
            .map(|guard| RoleId(*guard.value()))
    }

    pub fn tracking(&self) -> &OsuTracking {
        &self.data.osu_tracking
    }

    pub async fn retrieve_channel_history(
        &self,
        channel_id: ChannelId,
    ) -> Result<Vec<Message>, HttpError> {
        let req = self.http.channel_messages(channel_id).limit(50).unwrap();
        if let Some(earliest_cached) = self.cache.first_message(channel_id) {
            req.before(earliest_cached).await
        } else {
            req.await
        }
    }

    pub fn store_msg(&self, msg: MessageId) {
        self.data.msgs_to_minimize.insert(msg);
    }

    pub fn remove_msg(&self, msg: MessageId) -> bool {
        self.data.msgs_to_minimize.remove(&msg).is_some()
    }
}
