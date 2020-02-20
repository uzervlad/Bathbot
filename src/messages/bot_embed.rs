#![allow(clippy::too_many_arguments)]
#![allow(unused)]

use crate::{
    messages::{
        AuthorDescThumbData, CommonData, PPMissingData, ProfileData, ScoreMultiData,
        ScoreSingleData, SimulateData, WhatIfPPData,
    },
    util::{
        datetime::{date_to_string, how_long_ago, sec_to_minsec},
        numbers::{round, round_and_comma, with_comma_u64},
        osu::*,
    },
};

use itertools::Itertools;
use roppai::Oppai;
use rosu::models::{Beatmap, GameMod, GameMode, GameMods, Score, User};
use serenity::{builder::CreateEmbed, cache::CacheRwLock, utils::Colour};
use std::{cmp::Ordering::Equal, collections::HashMap, f32, u32};

pub enum BotEmbed<'d> {
    UserScoreSingle(&'d ScoreSingleData),
    UserScoreMulti(Box<ScoreMultiData>),
    AuthorDescThumb(AuthorDescThumbData),
    Profile(ProfileData),
    PPMissing(PPMissingData),
    WhatIfPP(WhatIfPPData),
    SimulateScore(&'d SimulateData),
    UserCommonScores(CommonData),
    //UserLeaderboard(Box<Beatmap>, Vec<(User, Score)>),
    UserScoreSingleMini(Box<ScoreSingleData>),
    SimulateScoreMini(Box<SimulateData>),
}

impl<'d, 'e> BotEmbed<'d> {
    pub fn create(self, e: &'e mut CreateEmbed) -> &'e mut CreateEmbed {
        e.color(Colour::DARK_GREEN);
        match self {
            BotEmbed::UserScoreSingle(data) => create_user_score_single(e, data),
            BotEmbed::UserScoreMulti(data) => create_user_score_multi(e, *data),
            BotEmbed::AuthorDescThumb(data) => create_author_desc_thumb(e, data),
            BotEmbed::Profile(data) => create_profile(e, data),
            BotEmbed::PPMissing(data) => create_pp_missing(e, data),
            BotEmbed::WhatIfPP(data) => create_whatif_pp(e, data),
            BotEmbed::SimulateScore(data) => create_simulation(e, data),
            BotEmbed::UserCommonScores(data) => create_common(e, data),
            //BotEmbed::UserLeaderboard(data) => create_leaderboard(e, data),
            BotEmbed::UserScoreSingleMini(data) => create_user_score_single_mini(e, *data),
            BotEmbed::SimulateScoreMini(data) => create_simulation_mini(e, *data),
        }
    }
}

fn create_simulation_mini(embed: &mut CreateEmbed, data: SimulateData) -> &mut CreateEmbed {
    let name = format!("{} ({})", data.grade_completion_mods, data.acc);
    let value = format!("{} [ {} ] {}", data.pp, data.combo, data.hits);
    let title = format!("{} [{}]", data.title, data.stars);
    embed
        .field(name, value, false)
        .thumbnail(&data.thumbnail)
        .url(&data.title_url)
        .title(title)
}

fn create_simulation<'d, 'e>(
    embed: &'e mut CreateEmbed,
    data: &'d SimulateData,
) -> &'e mut CreateEmbed {
    embed
        .title(&data.title)
        .url(&data.title_url)
        .thumbnail(&data.thumbnail)
        .footer(|f| f.icon_url(&data.footer_url).text(&data.footer_text))
        .fields(vec![
            ("Grade", &data.grade_completion_mods, true),
            ("Acc", &data.acc, true),
            ("Combo", &data.combo, true),
            ("PP", &data.pp, true),
            ("Hits", &data.hits, true),
            ("Map Info", &data.map_info, false),
        ])
}

fn create_user_score_single_mini(
    embed: &mut CreateEmbed,
    data: ScoreSingleData,
) -> &mut CreateEmbed {
    let name = format!(
        "{}\t{}\t({})\t{}",
        data.grade_completion_mods, data.score, data.acc, data.ago
    );
    let value = format!("{} [ {} ] {}", data.pp, data.combo, data.hits);
    let title = format!("{} [{}]", data.title, data.stars);
    embed
        .field(name, value, false)
        .thumbnail(&data.thumbnail)
        .title(title)
        .url(&data.title_url)
        .author(|a| {
            a.icon_url(data.author_icon)
                .url(data.author_url)
                .name(data.author_text)
        })
}

fn create_user_score_single<'d, 'e>(
    embed: &'e mut CreateEmbed,
    data: &'d ScoreSingleData,
) -> &'e mut CreateEmbed {
    if data.description.is_some() {
        embed.description(&data.description.as_ref().unwrap());
    }
    embed
        .title(&data.title)
        .url(&data.title_url)
        .timestamp(data.timestamp.clone())
        .thumbnail(&data.thumbnail)
        .footer(|f| f.icon_url(&data.footer_url).text(&data.footer_text))
        .fields(vec![
            ("Grade", &data.grade_completion_mods, true),
            ("Score", &data.score, true),
            ("Acc", &data.acc, true),
            ("PP", &data.pp, true),
            ("Combo", &data.combo, true),
            ("Hits", &data.hits, true),
            ("Map Info", &data.map_info, false),
        ])
        .author(|a| {
            a.icon_url(&data.author_icon)
                .url(&data.author_url)
                .name(&data.author_text)
        })
}

fn create_user_score_multi(embed: &mut CreateEmbed, data: ScoreMultiData) -> &mut CreateEmbed {
    embed
        .footer(|f| f.icon_url(&data.footer_url).text(&data.footer_text))
        .author(|a| {
            a.icon_url(&data.author_icon)
                .url(&data.author_url)
                .name(&data.author_text)
        })
        .title(data.title)
        .thumbnail(data.thumbnail)
        .url(data.title_url);
    if data.fields.is_empty() {
        embed.description("No scores found")
    } else {
        embed.fields(data.fields)
    }
}

fn create_author_desc_thumb(
    embed: &mut CreateEmbed,
    data: AuthorDescThumbData,
) -> &mut CreateEmbed {
    embed
        .thumbnail(&data.thumbnail)
        .description(&data.description)
        .author(|a| {
            a.icon_url(data.author_icon)
                .url(data.author_url)
                .name(data.author_text)
        })
}

fn create_profile(embed: &mut CreateEmbed, data: ProfileData) -> &mut CreateEmbed {
    embed
        .footer(|f| f.text(&data.footer_text))
        .author(|a| {
            a.icon_url(&data.author_icon)
                .url(&data.author_url)
                .name(&data.author_text)
        })
        .thumbnail(data.thumbnail)
        .fields(data.fields)
}

fn create_common(embed: &mut CreateEmbed, data: CommonData) -> &mut CreateEmbed {
    embed
        .description(data.description)
        .thumbnail("attachment://avatar_fuse.png")
}

fn create_whatif_pp(embed: &mut CreateEmbed, data: WhatIfPPData) -> &mut CreateEmbed {
    embed
        .thumbnail(&data.thumbnail)
        .description(&data.description)
        .title(&data.title)
        .author(|a| {
            a.icon_url(data.author_icon)
                .url(data.author_url)
                .name(data.author_text)
        })
}

fn create_pp_missing(embed: &mut CreateEmbed, data: PPMissingData) -> &mut CreateEmbed {
    embed
        .thumbnail(&data.thumbnail)
        .description(&data.description)
        .title(&data.title)
        .author(|a| {
            a.icon_url(data.author_icon)
                .url(data.author_url)
                .name(data.author_text)
        })
}

// TODO
fn create_leaderboard(embed: &mut CreateEmbed) -> &mut CreateEmbed {
    embed
}
