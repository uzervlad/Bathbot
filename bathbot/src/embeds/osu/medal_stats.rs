use std::fmt::{Display, Formatter, Result as FmtResult, Write};

use bathbot_macros::EmbedData;
use hashbrown::HashMap;
use rosu_v2::prelude::MedalCompact;
use twilight_model::channel::embed::EmbedField;

use crate::{
    custom_client::{MedalGroup, MEDAL_GROUPS},
    embeds::attachment,
    manager::redis::{osu::User, RedisData},
    util::{
        builder::{AuthorBuilder, FooterBuilder},
        constants::OSU_BASE,
        hasher::IntHasher,
        numbers::round,
        osu::flag_url,
        CowUtils,
    },
};

#[derive(EmbedData)]
pub struct MedalStatsEmbed {
    author: AuthorBuilder,
    fields: Vec<EmbedField>,
    footer: FooterBuilder,
    image: String,
    thumbnail: String,
}

impl MedalStatsEmbed {
    pub fn new(
        user: &RedisData<User>,
        user_medals: &[MedalCompact],
        medals: &HashMap<u32, (String, MedalGroup), IntHasher>,
        rarest: Option<MedalCompact>,
        with_graph: bool,
    ) -> Self {
        let completion = round(100.0 * user_medals.len() as f32 / medals.len() as f32);

        let mut fields = fields![
            "Medals", format!("{} / {}", user_medals.len(), medals.len()), true;
            "Completion", format!("{completion}%"), true;
        ];

        let oldest = user_medals.first();
        let newest = user_medals.last();

        if oldest.or(newest).or(rarest.as_ref()).is_some() {
            let mut value = String::with_capacity(128);

            if let Some(((name, _), date)) =
                oldest.and_then(|medal| Some((medals.get(&medal.medal_id)?, medal.achieved_at)))
            {
                let _ = writeln!(
                    value,
                    "👴 `Oldest` [{name}]({url}) <t:{timestamp}:d>",
                    url = MedalUrl { name },
                    timestamp = date.unix_timestamp()
                );
            }

            if let Some(((name, _), date)) =
                newest.and_then(|medal| Some((medals.get(&medal.medal_id)?, medal.achieved_at)))
            {
                let _ = writeln!(
                    value,
                    "👶 `Newest` [{name}]({url}) <t:{timestamp}:d>",
                    url = MedalUrl { name },
                    timestamp = date.unix_timestamp()
                );
            }

            if let Some(((name, _), date)) =
                rarest.and_then(|medal| Some((medals.get(&medal.medal_id)?, medal.achieved_at)))
            {
                let _ = writeln!(
                    value,
                    "💎 `Rarest` [{name}]({url}) <t:{timestamp}:d>",
                    url = MedalUrl { name },
                    timestamp = date.unix_timestamp()
                );
            }

            fields![fields { "Corner stone medals", value, false }];
        }

        if !user_medals.is_empty() {
            let mut counts = HashMap::new();

            // Count groups for all medals
            for (_, grouping) in medals.values() {
                let (total, _) = counts.entry(grouping.as_str()).or_insert((0, 0));
                *total += 1;
            }

            // Count groups for owned medals
            for medal in user_medals.iter() {
                let entry = medals
                    .get(&medal.medal_id)
                    .and_then(|(_, grouping)| counts.get_mut(grouping.as_str()));

                if let Some((_, owned)) = entry {
                    *owned += 1;
                }
            }

            // Add to fields
            MEDAL_GROUPS
                .iter()
                .map(|group| group.as_str())
                .for_each(|group| {
                    if let Some((total, owned)) = counts.get(group) {
                        let value = format!("{owned} / {total}");
                        fields![fields { group.to_string(), value, true }];
                    }
                });
        }

        let (country_code, username, user_id) = match user {
            RedisData::Original(user) => {
                let country_code = user.country_code.as_str();
                let username = user.username.as_str();
                let user_id = user.user_id;

                (country_code, username, user_id)
            }
            RedisData::Archived(user) => {
                let country_code = user.country_code.as_str();
                let username = user.username.as_str();
                let user_id = user.user_id;

                (country_code, username, user_id)
            }
        };

        let author = AuthorBuilder::new(username)
            .url(format!("{OSU_BASE}u/{user_id}"))
            .icon_url(flag_url(country_code));

        let footer = FooterBuilder::new("Check osekai.net for more info");

        let image = with_graph
            .then(|| attachment("medal_graph.png"))
            .unwrap_or_default();

        Self {
            image,
            author,
            fields,
            footer,
            thumbnail: user.avatar_url().to_owned(),
        }
    }
}

struct MedalUrl<'n> {
    name: &'n str,
}

impl Display for MedalUrl<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(
            f,
            "https://osekai.net/medals/?medal={}",
            self.name.cow_replace(' ', "+").cow_replace(',', "%2C")
        )
    }
}