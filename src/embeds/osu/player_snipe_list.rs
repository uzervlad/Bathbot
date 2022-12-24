use std::{collections::BTreeMap, fmt::Write};

use command_macros::EmbedData;
use hashbrown::HashMap;
use rosu_v2::prelude::{Beatmap, User};

use crate::{
    core::Context,
    custom_client::SnipeScore,
    embeds::osu,
    pagination::Pages,
    pp::PpCalculator,
    util::{
        builder::{AuthorBuilder, FooterBuilder},
        constants::OSU_BASE,
        datetime::how_long_ago_dynamic,
        hasher::IntHasher,
        numbers::{round, with_comma_int},
        CowUtils,
    },
};

#[derive(EmbedData)]
pub struct PlayerSnipeListEmbed {
    author: AuthorBuilder,
    description: String,
    footer: FooterBuilder,
    thumbnail: String,
}

impl PlayerSnipeListEmbed {
    pub async fn new(
        user: &User,
        scores: &BTreeMap<usize, SnipeScore>,
        maps: &HashMap<u32, Beatmap, IntHasher>,
        total: usize,
        ctx: &Context,
        pages: &Pages,
    ) -> Self {
        if scores.is_empty() {
            return Self {
                author: author!(user),
                thumbnail: user.avatar_url.to_owned(),
                footer: FooterBuilder::new("Page 1/1 ~ Total #1 scores: 0"),
                description: "No scores were found".to_owned(),
            };
        }

        let page = pages.curr_page();
        let pages = pages.last_page();
        let index = (page - 1) * 5;
        let entries = scores.range(index..index + 5);
        let mut description = String::with_capacity(1024);

        // TODO: update formatting
        for (idx, score) in entries {
            let map = maps.get(&score.map.map_id).expect("missing map");

            let max_pp = match PpCalculator::new(ctx, map.map_id).await {
                Ok(calc) => Some(calc.mods(score.mods.unwrap_or_default()).max_pp() as f32),
                Err(err) => {
                    warn!("{:?}", err.wrap_err("Failed to get pp calculator"));

                    None
                }
            };

            let pp = osu::get_pp(score.pp, max_pp);
            let n300 = map.count_objects()
                - score.count_100.unwrap_or(0)
                - score.count_50.unwrap_or(0)
                - score.count_miss.unwrap_or(0);

            let title = map
                .mapset
                .as_ref()
                .unwrap()
                .title
                .as_str()
                .cow_escape_markdown();

            let _ = write!(
                description,
                "**{idx}. [{title} [{version}]]({OSU_BASE}b/{id}) {mods}** [{stars:.2}★]\n\
                {pp} ~ ({acc}%) ~ {score}\n{{{n300}/{n100}/{n50}/{nmiss}}}",
                idx = idx + 1,
                version = map.version.as_str().cow_escape_markdown(),
                id = score.map.map_id,
                mods = osu::get_mods(score.mods.unwrap_or_default()),
                stars = score.stars,
                acc = round(score.accuracy),
                score = with_comma_int(score.score),
                n100 = score.count_100.unwrap_or(0),
                n50 = score.count_50.unwrap_or(0),
                nmiss = score.count_miss.unwrap_or(0),
            );

            if let Some(ref date) = score.date_set {
                let _ = write!(description, " ~ {ago}", ago = how_long_ago_dynamic(date));
            }

            description.push('\n');
        }

        let footer = FooterBuilder::new(format!("Page {page}/{pages} ~ Total scores: {total}"));

        Self {
            author: author!(user),
            description,
            footer,
            thumbnail: user.avatar_url.to_owned(),
        }
    }
}
