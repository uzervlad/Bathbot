use rosu_v2::prelude::User;
use twilight_model::channel::Message;

use crate::{commands::osu::MedalEntryList, embeds::MedalsListEmbed, BotResult};

use super::{Pages, Pagination};

pub struct MedalsListPagination {
    msg: Message,
    pages: Pages,
    user: User,
    acquired: (usize, usize),
    medals: Vec<MedalEntryList>,
}

impl MedalsListPagination {
    pub fn new(
        msg: Message,
        user: User,
        medals: Vec<MedalEntryList>,
        acquired: (usize, usize),
    ) -> Self {
        Self {
            pages: Pages::new(10, medals.len()),
            msg,
            user,
            medals,
            acquired,
        }
    }
}

#[async_trait]
impl Pagination for MedalsListPagination {
    type PageData = MedalsListEmbed;

    fn msg(&self) -> &Message {
        &self.msg
    }

    fn pages(&self) -> Pages {
        self.pages
    }

    fn pages_mut(&mut self) -> &mut Pages {
        &mut self.pages
    }

    fn single_step(&self) -> usize {
        self.pages.per_page
    }

    async fn build_page(&mut self) -> BotResult<Self::PageData> {
        let page = self.page();
        let idx = (page - 1) * self.pages.per_page;
        let limit = self.medals.len().min(idx + self.pages.per_page);

        Ok(MedalsListEmbed::new(
            &self.user,
            &self.medals[idx..limit],
            self.acquired,
            (page, self.pages.total_pages),
        ))
    }
}