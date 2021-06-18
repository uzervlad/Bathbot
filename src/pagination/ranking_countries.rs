use super::{Pages, Pagination};

use crate::{embeds::RankingCountriesEmbed, BotResult, Context};

use async_trait::async_trait;
use rosu_v2::prelude::{CountryRanking, GameMode};
use std::{collections::BTreeMap, sync::Arc};
use twilight_model::channel::Message;

pub struct RankingCountriesPagination {
    msg: Message,
    pages: Pages,
    ctx: Arc<Context>,
    mode: GameMode,
    countries: BTreeMap<usize, CountryRanking>,
    total: usize,
}

impl RankingCountriesPagination {
    pub fn new(
        msg: Message,
        mode: GameMode,
        ctx: Arc<Context>,
        total: usize,
        countries: BTreeMap<usize, CountryRanking>,
    ) -> Self {
        Self {
            pages: Pages::new(15, total),
            msg,
            ctx,
            mode,
            countries,
            total,
        }
    }
}

#[async_trait]
impl Pagination for RankingCountriesPagination {
    type PageData = RankingCountriesEmbed;

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
        let count = self
            .countries
            .range(self.pages.index..self.pages.index + self.pages.per_page)
            .count();

        if count < self.pages.per_page && count < self.total - self.pages.index {
            // * If the amount of countries changes to 240-255,
            // * two request will need to be done to skip to the end
            let page = match self.pages.index {
                45 => 2,
                90 if !self.countries.contains_key(&90) => 2, // when going back to front
                90 | 135 => 3,
                150 => 4,
                195 if !self.countries.contains_key(&195) => 4, // when going back to front
                195 | 225 => 5,
                _ => unreachable!("unexpected page index {}", self.pages.index),
            };

            let offset = page - 1;

            let mut ranking = self
                .ctx
                .osu()
                .country_rankings(self.mode)
                .page(page as u32)
                .await?;

            let iter = ranking
                .ranking
                .drain(..)
                .enumerate()
                .map(|(i, country)| (offset * 50 + i, country));

            self.countries.extend(iter);
        }

        Ok(RankingCountriesEmbed::new(
            self.mode,
            &self.countries,
            (self.page(), self.pages.total_pages),
        ))
    }
}