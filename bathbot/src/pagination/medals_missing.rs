use bathbot_macros::pagination;
use bathbot_model::rosu_v2::user::User;
use twilight_model::channel::message::embed::Embed;

use super::Pages;
use crate::{
    commands::osu::MedalType,
    embeds::{EmbedData, MedalsMissingEmbed},
    manager::redis::RedisData,
};

#[pagination(per_page = 15, entries = "medals")]
pub struct MedalsMissingPagination {
    user: RedisData<User>,
    medals: Vec<MedalType>,
    medal_count: (usize, usize),
}

impl MedalsMissingPagination {
    pub fn build_page(&mut self, pages: &Pages) -> Embed {
        let idx = pages.index();
        let limit = self.medals.len().min(idx + pages.per_page());

        let embed = MedalsMissingEmbed::new(
            &self.user,
            &self.medals[idx..limit],
            self.medal_count,
            limit == self.medals.len(),
            pages,
        );

        embed.build()
    }
}
