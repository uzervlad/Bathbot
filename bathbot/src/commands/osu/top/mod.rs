use std::{
    borrow::Cow,
    cmp::{Ordering, Reverse},
    collections::HashMap,
    fmt::Write,
    mem,
    sync::Arc,
};

use bathbot_macros::{command, HasMods, HasName, SlashCommand};
use bathbot_model::{OsuTrackerMapsetEntry, ScoreSlim};
use bathbot_psql::model::configs::{GuildConfig, ListSize, MinimizedPp, ScoreSize};
use bathbot_util::{
    constants::{GENERAL_ISSUE, OSUTRACKER_ISSUE, OSU_API_ISSUE},
    matcher,
    numbers::round,
    osu::ModSelection,
    CowUtils, IntHasher, MessageBuilder,
};
use eyre::{Report, Result};
use rkyv::{Deserialize, Infallible};
use rosu_v2::{
    prelude::{
        GameMode, GameMods, Grade, OsuError,
        RankStatus::{Approved, Loved, Qualified, Ranked},
        Score,
    },
    request::UserId,
};
use tokio::time::{sleep, Duration};
use twilight_interactions::command::{CommandModel, CommandOption, CreateCommand, CreateOption};
use twilight_model::id::{marker::UserMarker, Id};

use crate::{
    commands::{GameModeOption, GradeOption},
    core::commands::{prefix::Args, CommandOrigin},
    embeds::TopSingleEmbed,
    manager::{
        redis::{
            osu::{User, UserArgs},
            RedisData,
        },
        OsuMap,
    },
    pagination::{TopCondensedPagination, TopPagination, TopSinglePagination},
    util::{
        interaction::InteractionCommand,
        query::{FilterCriteria, Searchable},
        ChannelExt, InteractionCommandExt, MessageExt,
    },
    Context,
};

pub use self::{if_::*, old::*};

use super::{require_link, user_not_found, HasMods, ModsResult, ScoreOrder};

mod if_;
mod old;

#[derive(CommandModel, CreateCommand, HasMods, SlashCommand)]
#[command(name = "top")]
/// Display the user's current top100
pub struct Top {
    /// Specify a gamemode
    mode: Option<GameModeOption>,
    /// Specify a username
    name: Option<String>,
    #[command(help = "Choose how the scores should be ordered, defaults to `pp`.")]
    /// Choose how the scores should be ordered
    sort: Option<TopScoreOrder>,
    #[command(help = "Filter out all scores that don't match the specified mods.\n\
        Mods must be given as `+mods` for included mods, `+mods!` for exact mods, \
        or `-mods!` for excluded mods.\n\
        Examples:\n\
        - `+hd`: Scores must have at least `HD` but can also have more other mods\n\
        - `+hdhr!`: Scores must have exactly `HDHR`\n\
        - `-ezhd!`: Scores must have neither `EZ` nor `HD` e.g. `HDDT` would get filtered out\n\
        - `-nm!`: Scores can not be nomod so there must be any other mod")]
    /// Specify mods (`+mods` for included, `+mods!` for exact, `-mods!` for excluded)
    mods: Option<String>,
    #[command(min_value = 1, max_value = 100)]
    /// Choose a specific score index
    index: Option<u32>,
    #[command(
        help = "Instead of specifying an osu! username with the `name` option, \
        you can use this option to choose a discord user.\n\
        Only works on users who have used the `/link` command."
    )]
    /// Specify a linked discord user
    discord: Option<Id<UserMarker>>,
    /// Reverse the resulting score list
    reverse: Option<bool>,
    #[command(
        help = "Filter out scores similarly as you filter maps in osu! itself.\n\
        You can specify the artist, creator, difficulty, title, or limit values such as \
        ar, cs, hp, od, bpm, length, or stars like for example `fdfd ar>10 od>=9`.\n\
        While ar & co will be adjusted to mods, stars will not."
    )]
    /// Specify a search query containing artist, difficulty, AR, BPM, ...
    query: Option<String>,
    /// Consider only scores with this grade
    grade: Option<GradeOption>,
    #[command(help = "Specify if you want to filter out farm maps.\n\
        A map counts as farmy if its mapset appears in the top 727 \
        sets based on how often the set is in people's top100 scores.\n\
        The list of mapsets can be checked with `/popular mapsets` or \
        on [here](https://osutracker.com/stats)")]
    /// Specify if you want to filter out farm maps
    farm: Option<FarmFilter>,
    /// Filter out all scores that don't have a perfect combo
    perfect_combo: Option<bool>,
    #[command(help = "Size of the embed.\n\
      `Condensed` shows 10 scores, `Detailed` shows 5, and `Single` shows 1.\n\
      The default can be set with the `/config` command.")]
    /// Size of the embed
    size: Option<ListSize>,
}

#[derive(Copy, Clone, CommandOption, CreateOption, Eq, PartialEq)]
pub enum TopScoreOrder {
    #[option(name = "Accuracy", value = "acc")]
    Acc,
    #[option(name = "BPM", value = "bpm")]
    Bpm,
    #[option(name = "Combo", value = "combo")]
    Combo,
    #[option(name = "Date", value = "date")]
    Date,
    #[option(name = "Common farm", value = "farm")]
    Farm,
    #[option(name = "Length", value = "len")]
    Length,
    #[option(name = "Map ranked date", value = "ranked_date")]
    RankedDate,
    #[option(name = "Misses", value = "miss")]
    Misses,
    #[option(name = "PP", value = "pp")]
    Pp,
    #[option(name = "Score", value = "score")]
    Score,
    #[option(name = "Stars", value = "stars")]
    Stars,
}

impl Default for TopScoreOrder {
    fn default() -> Self {
        Self::Pp
    }
}

impl From<ScoreOrder> for TopScoreOrder {
    fn from(sort_by: ScoreOrder) -> Self {
        match sort_by {
            ScoreOrder::Acc => Self::Acc,
            ScoreOrder::Bpm => Self::Bpm,
            ScoreOrder::Combo => Self::Combo,
            ScoreOrder::Date => Self::Date,
            ScoreOrder::Length => Self::Length,
            ScoreOrder::Misses => Self::Misses,
            ScoreOrder::Pp => Self::Pp,
            ScoreOrder::RankedDate => Self::RankedDate,
            ScoreOrder::Score => Self::Score,
            ScoreOrder::Stars => Self::Stars,
        }
    }
}

#[derive(CommandOption, CreateOption)]
pub enum FarmFilter {
    #[option(name = "No farm", value = "no_farm")]
    NoFarm,
    #[option(name = "Only farm", value = "only_farm")]
    OnlyFarm,
}

#[command]
#[desc("Display a user's top plays")]
#[help(
    "Display a user's top plays.\n\
     Mods can be specified through the usual `+_`, `+_!`, `-_!` syntax.\n\
     There are also multiple options you can set by specifying `key=value`.\n\
     These are the keys with their values:\n\
     - `acc`: single number or two numbers of the form `a..b` e.g. `acc=97.5..98`\n\
     - `combo`: single integer or two integers of the form `a..b` e.g. `combo=500..1234`\n\
     - `grade`: `SS`, `S`, `A`, `B`, `C`, or `D`\n\
     - `sort`: `acc`, `combo`, `date` (= `rb` command), `length`, or `position` (default)\n\
     - `reverse`: `true` or `false` (default)\n\
     \n\
     Instead of showing the scores in a list, you can also __show a single score__ by \
     specifying a number right after the command, e.g. `<top2 badewanne3`."
)]
#[usage(
    "[username] [mods] [acc=number[..number]] [combo=integer[..integer]] \
    [grade=SS/S/A/B/C/D] [sort=acc/combo/date/length/position] [reverse=true/false]"
)]
#[examples(
    "badewanne3 acc=97.34..99.5 grade=A +hdhr sort=combo",
    "vaxei -dt! combo=1234 sort=length",
    "peppy combo=200..500 grade=B reverse=true"
)]
#[aliases("topscores", "toposu", "topstd", "topstandard", "topo", "tops")]
#[group(Osu)]
async fn prefix_top(ctx: Arc<Context>, msg: &Message, args: Args<'_>) -> Result<()> {
    match TopArgs::args(None, args) {
        Ok(args) => top(ctx, msg.into(), args).await,
        Err(content) => {
            msg.error(&ctx, content).await?;

            Ok(())
        }
    }
}

#[command]
#[desc("Display a user's top mania plays")]
#[help(
    "Display a user's top mania plays.\n\
    Mods can be specified through the usual `+_`, `+_!`, `-_!` syntax.\n\
    There are also multiple options you can set by specifying `key=value`.\n\
    These are the keys with their values:\n\
    - `acc`: single number or two numbers of the form `a..b` e.g. `acc=97.5..98`\n\
    - `combo`: single integer or two integers of the form `a..b` e.g. `combo=500..1234`\n\
    - `grade`: `SS`, `S`, `A`, `B`, `C`, or `D`\n\
    - `sort`: `acc`, `combo`, `date` (= `rbm` command), `length`, or `position` (default)\n\
    - `reverse`: `true` or `false` (default)\n\
    \n\
    Instead of showing the scores in a list, you can also __show a single score__ by \
    specifying a number right after the command, e.g. `<topm2 badewanne3`."
)]
#[usage(
    "[username] [mods] [acc=number[..number]] [combo=integer[..integer]] \
    [grade=SS/S/A/B/C/D] [sort=acc/combo/date/length/position] [reverse=true/false]"
)]
#[examples(
    "badewanne3 acc=97.34..99.5 grade=A +hdhr sort=combo",
    "vaxei -dt! combo=1234 sort=length",
    "peppy combo=200..500 grade=B reverse=true"
)]
#[alias("topm")]
#[group(Mania)]
async fn prefix_topmania(ctx: Arc<Context>, msg: &Message, args: Args<'_>) -> Result<()> {
    match TopArgs::args(Some(GameMode::Mania), args) {
        Ok(args) => top(ctx, msg.into(), args).await,
        Err(content) => {
            msg.error(&ctx, content).await?;

            Ok(())
        }
    }
}

#[command]
#[desc("Display a user's top taiko plays")]
#[help(
    "Display a user's top taiko plays.\n\
    Mods can be specified through the usual `+_`, `+_!`, `-_!` syntax.\n\
    There are also multiple options you can set by specifying `key=value`.\n\
    These are the keys with their values:\n\
    - `acc`: single number or two numbers of the form `a..b` e.g. `acc=97.5..98`\n\
    - `combo`: single integer or two integers of the form `a..b` e.g. `combo=500..1234`\n\
    - `grade`: `SS`, `S`, `A`, `B`, `C`, or `D`\n\
    - `sort`: `acc`, `combo`, `date` (= `rbt` command), `length`, or `position` (default)\n\
    - `reverse`: `true` or `false` (default)\n\
    \n\
    Instead of showing the scores in a list, you can also __show a single score__ by \
    specifying a number right after the command, e.g. `<topt2 badewanne3`."
)]
#[usage(
    "[username] [mods] [acc=number[..number]] [combo=integer[..integer]] \
    [grade=SS/S/A/B/C/D] [sort=acc/combo/date/length/position] [reverse=true/false]"
)]
#[examples(
    "badewanne3 acc=97.34..99.5 grade=A +hdhr sort=combo",
    "vaxei -dt! combo=1234 sort=length",
    "peppy combo=200..500 grade=B reverse=true"
)]
#[alias("topt")]
#[group(Taiko)]
async fn prefix_toptaiko(ctx: Arc<Context>, msg: &Message, args: Args<'_>) -> Result<()> {
    match TopArgs::args(Some(GameMode::Taiko), args) {
        Ok(args) => top(ctx, msg.into(), args).await,
        Err(content) => {
            msg.error(&ctx, content).await?;

            Ok(())
        }
    }
}

#[command]
#[desc("Display a user's top ctb plays")]
#[help(
    "Display a user's top ctb plays.\n\
    Mods can be specified through the usual `+_`, `+_!`, `-_!` syntax.\n\
    There are also multiple options you can set by specifying `key=value`.\n\
    These are the keys with their values:\n\
    - `acc`: single number or two numbers of the form `a..b` e.g. `acc=97.5..98`\n\
    - `combo`: single integer or two integers of the form `a..b` e.g. `combo=500..1234`\n\
    - `grade`: `SS`, `S`, `A`, `B`, `C`, or `D`\n\
    - `sort`: `acc`, `combo`, `date` (= `rbc` command), `length`, or `position` (default)\n\
    - `reverse`: `true` or `false` (default)\n\
    \n\
    Instead of showing the scores in a list, you can also __show a single score__ by \
    specifying a number right after the command, e.g. `<topc2 badewanne3`."
)]
#[usage(
    "[username] [mods] [acc=number[..number]] [combo=integer[..integer]] \
   [grade=SS/S/A/B/C/D] [sort=acc/combo/date/length/position] [reverse=true/false]"
)]
#[examples(
    "badewanne3 acc=97.34..99.5 grade=A +hdhr sort=combo",
    "vaxei -dt! combo=1234 sort=length",
    "peppy combo=200..500 grade=B reverse=true"
)]
#[alias("topc", "topcatch", "topcatchthebeat")]
#[group(Catch)]
async fn prefix_topctb(ctx: Arc<Context>, msg: &Message, args: Args<'_>) -> Result<()> {
    match TopArgs::args(Some(GameMode::Catch), args) {
        Ok(args) => top(ctx, msg.into(), args).await,
        Err(content) => {
            msg.error(&ctx, content).await?;

            Ok(())
        }
    }
}

#[command]
#[desc("Sort a user's top plays by date")]
#[help(
    "Display a user's most recent top plays.\n\
    Mods can be specified through the usual `+_`, `+_!`, `-_!` syntax.\n\
    There are also multiple options you can set by specifying `key=value`.\n\
    These are the keys with their values:\n\
    - `acc`: single number or two numbers of the form `a..b` e.g. `acc=97.5..98`\n\
    - `combo`: single integer or two integers of the form `a..b` e.g. `combo=500..1234`\n\
    - `grade`: `SS`, `S`, `A`, `B`, `C`, or `D`\n\
    - `reverse`: `true` or `false` (default)\n\
    \n\
    Instead of showing the scores in a list, you can also __show a single score__ by \
    specifying a number right after the command, e.g. `<rb2 badewanne3`."
)]
#[usage(
    "[username] [mods] [acc=number[..number]] [combo=integer[..integer]] [grade=SS/S/A/B/C/D] [reverse=true/false]"
)]
#[examples(
    "badewanne3 acc=97.34..99.5 grade=A +hdhr",
    "vaxei -dt! combo=1234",
    "peppy combo=200..500 grade=B reverse=true"
)]
#[alias(
    "rb",
    "rbo",
    "rbs",
    "recentbestosu",
    "recentbeststd",
    "recentbeststandard"
)]
#[group(Osu)]
async fn prefix_recentbest(ctx: Arc<Context>, msg: &Message, args: Args<'_>) -> Result<()> {
    match TopArgs::args(None, args) {
        Ok(mut args) => {
            args.sort_by = TopScoreOrder::Date;

            top(ctx, msg.into(), args).await
        }
        Err(content) => {
            msg.error(&ctx, content).await?;

            Ok(())
        }
    }
}

#[command]
#[desc("Sort a user's top mania plays by date")]
#[help(
    "Display a user's most recent top mania plays.\n\
    Mods can be specified through the usual `+_`, `+_!`, `-_!` syntax.\n\
    There are also multiple options you can set by specifying `key=value`.\n\
    These are the keys with their values:\n\
    - `acc`: single number or two numbers of the form `a..b` e.g. `acc=97.5..98`\n\
    - `combo`: single integer or two integers of the form `a..b` e.g. `combo=500..1234`\n\
    - `grade`: `SS`, `S`, `A`, `B`, `C`, or `D`\n\
    - `reverse`: `true` or `false` (default)\n\
    \n\
    Instead of showing the scores in a list, you can also __show a single score__ by \
    specifying a number right after the command, e.g. `<rbm2 badewanne3`."
)]
#[usage(
    "[username] [mods] [acc=number[..number]] [combo=integer[..integer]] [grade=SS/S/A/B/C/D] [reverse=true/false]"
)]
#[examples(
    "badewanne3 acc=97.34..99.5 grade=A +hdhr",
    "vaxei -dt! combo=1234",
    "peppy combo=200..500 grade=B reverse=true"
)]
#[alias("rbm")]
#[group(Mania)]
async fn prefix_recentbestmania(ctx: Arc<Context>, msg: &Message, args: Args<'_>) -> Result<()> {
    match TopArgs::args(Some(GameMode::Mania), args) {
        Ok(mut args) => {
            args.sort_by = TopScoreOrder::Date;

            top(ctx, msg.into(), args).await
        }
        Err(content) => {
            msg.error(&ctx, content).await?;

            Ok(())
        }
    }
}

#[command]
#[desc("Sort a user's top taiko plays by date")]
#[help(
    "Display a user's most recent top taiko plays.\n\
    Mods can be specified through the usual `+_`, `+_!`, `-_!` syntax.\n\
    There are also multiple options you can set by specifying `key=value`.\n\
    These are the keys with their values:\n\
    - `acc`: single number or two numbers of the form `a..b` e.g. `acc=97.5..98`\n\
    - `combo`: single integer or two integers of the form `a..b` e.g. `combo=500..1234`\n\
    - `grade`: `SS`, `S`, `A`, `B`, `C`, or `D`\n\
    - `reverse`: `true` or `false` (default)\n\
    \n\
    Instead of showing the scores in a list, you can also __show a single score__ by \
    specifying a number right after the command, e.g. `<rbt2 badewanne3`."
)]
#[usage(
    "[username] [mods] [acc=number[..number]] [combo=integer[..integer]] [grade=SS/S/A/B/C/D] [reverse=true/false]"
)]
#[examples(
    "badewanne3 acc=97.34..99.5 grade=A +hdhr",
    "vaxei -dt! combo=1234",
    "peppy combo=200..500 grade=B reverse=true"
)]
#[alias("rbt")]
#[group(Taiko)]
async fn prefix_recentbesttaiko(ctx: Arc<Context>, msg: &Message, args: Args<'_>) -> Result<()> {
    match TopArgs::args(Some(GameMode::Taiko), args) {
        Ok(mut args) => {
            args.sort_by = TopScoreOrder::Date;

            top(ctx, msg.into(), args).await
        }
        Err(content) => {
            msg.error(&ctx, content).await?;

            Ok(())
        }
    }
}

#[command]
#[desc("Sort a user's top ctb plays by date")]
#[help(
    "Display a user's most recent top ctb plays.\n\
    Mods can be specified through the usual `+_`, `+_!`, `-_!` syntax.\n\
    There are also multiple options you can set by specifying `key=value`.\n\
    These are the keys with their values:\n\
    - `acc`: single number or two numbers of the form `a..b` e.g. `acc=97.5..98`\n\
    - `combo`: single integer or two integers of the form `a..b` e.g. `combo=500..1234`\n\
    - `grade`: `SS`, `S`, `A`, `B`, `C`, or `D`\n\
    - `reverse`: `true` or `false` (default)\n\
    \n\
    Instead of showing the scores in a list, you can also __show a single score__ by \
    specifying a number right after the command, e.g. `<rbc2 badewanne3`."
)]
#[usage(
    "[username] [mods] [acc=number[..number]] [combo=integer[..integer]] [grade=SS/S/A/B/C/D] [reverse=true/false]"
)]
#[examples(
    "badewanne3 acc=97.34..99.5 grade=A +hdhr",
    "vaxei -dt! combo=1234",
    "peppy combo=200..500 grade=B reverse=true"
)]
#[alias("rbc")]
#[group(Catch)]
async fn prefix_recentbestctb(ctx: Arc<Context>, msg: &Message, args: Args<'_>) -> Result<()> {
    match TopArgs::args(Some(GameMode::Catch), args) {
        Ok(mut args) => {
            args.sort_by = TopScoreOrder::Date;

            top(ctx, msg.into(), args).await
        }
        Err(content) => {
            msg.error(&ctx, content).await?;

            Ok(())
        }
    }
}

async fn slash_top(ctx: Arc<Context>, mut command: InteractionCommand) -> Result<()> {
    let args = Top::from_interaction(command.input_data())?;

    match TopArgs::try_from(args) {
        Ok(args) => top(ctx, (&mut command).into(), args).await,
        Err(content) => {
            command.error(&ctx, content).await?;

            Ok(())
        }
    }
}

#[derive(HasName)]
pub struct TopArgs<'a> {
    pub name: Option<Cow<'a, str>>,
    pub discord: Option<Id<UserMarker>>,
    pub mode: Option<GameMode>,
    pub mods: Option<ModSelection>,
    pub min_acc: Option<f32>,
    pub max_acc: Option<f32>,
    pub min_combo: Option<u32>,
    pub max_combo: Option<u32>,
    pub grade: Option<Grade>,
    pub sort_by: TopScoreOrder,
    pub reverse: bool,
    pub perfect_combo: Option<bool>,
    pub index: Option<usize>,
    pub query: Option<String>,
    pub farm: Option<FarmFilter>,
    pub size: Option<ListSize>,
    pub has_dash_r: bool,
    pub has_dash_p_or_i: bool,
}

impl<'m> TopArgs<'m> {
    pub const ERR_PARSE_MODS: &'static str = "Failed to parse mods.\n\
        If you want included mods, specify it e.g. as `+hrdt`.\n\
        If you want exact mods, specify it e.g. as `+hdhr!`.\n\
        And if you want to exclude mods, specify it e.g. as `-hdnf!`.";

    const ERR_PARSE_ACC: &'static str = "Failed to parse `accuracy`.\n\
        Must be either decimal number \
        or two decimal numbers of the form `a..b` e.g. `97.5..98.5`.";

    const ERR_PARSE_COMBO: &'static str = "Failed to parse `combo`.\n\
        Must be either a positive integer \
        or two positive integers of the form `a..b` e.g. `501..1234`.";

    fn args(mode: Option<GameMode>, args: Args<'m>) -> Result<Self, Cow<'static, str>> {
        let mut name = None;
        let mut discord = None;
        let mut mods = None;
        let mut acc_min = None;
        let mut acc_max = None;
        let mut combo_min = None;
        let mut combo_max = None;
        let mut grade = None;
        let mut sort_by = None;
        let mut reverse = None;
        let mut has_dash_r = None;
        let mut has_dash_p_or_i = None;
        let num = args.num;

        for arg in args.map(|arg| arg.cow_to_ascii_lowercase()) {
            if arg.as_ref() == "-r" {
                has_dash_r = Some(true);
            } else if matches!(arg.as_ref(), "-p" | "-i") {
                has_dash_p_or_i = Some(true);
            } else if let Some(idx) = arg.find('=').filter(|&i| i > 0) {
                let key = &arg[..idx];
                let value = arg[idx + 1..].trim_end();

                match key {
                    "acc" | "accuracy" | "a" => match value.find("..") {
                        Some(idx) => {
                            let bot = &value[..idx];
                            let top = &value[idx + 2..];

                            let mut min = if bot.is_empty() {
                                0.0
                            } else if let Ok(num) = bot.parse::<f32>() {
                                num.clamp(0.0, 100.0)
                            } else {
                                return Err(Self::ERR_PARSE_ACC.into());
                            };

                            let mut max = if top.is_empty() {
                                100.0
                            } else if let Ok(num) = top.parse::<f32>() {
                                num.clamp(0.0, 100.0)
                            } else {
                                return Err(Self::ERR_PARSE_ACC.into());
                            };

                            if min > max {
                                mem::swap(&mut min, &mut max);
                            }

                            acc_min = Some(min);
                            acc_max = Some(max);
                        }
                        None => match value.parse() {
                            Ok(num) => acc_min = Some(num),
                            Err(_) => return Err(Self::ERR_PARSE_ACC.into()),
                        },
                    },
                    "combo" | "c" => match value.find("..") {
                        Some(idx) => {
                            let bot = &value[..idx];
                            let top = &value[idx + 2..];

                            let mut min = if bot.is_empty() {
                                0
                            } else if let Ok(num) = bot.parse() {
                                num
                            } else {
                                return Err(Self::ERR_PARSE_COMBO.into());
                            };

                            let mut max = top.parse().ok();

                            if let Some(ref mut max) = max {
                                if min > *max {
                                    mem::swap(&mut min, max);
                                }
                            }

                            combo_min = Some(min);
                            combo_max = max;
                        }
                        None => match value.parse() {
                            Ok(num) => combo_min = Some(num),
                            Err(_) => return Err(Self::ERR_PARSE_COMBO.into()),
                        },
                    },
                    "grade" | "g" => match value.parse::<GradeOption>() {
                        Ok(grade_) => grade = Some(grade_.into()),
                        Err(content) => return Err(content.into()),
                    },
                    "sort" | "s" | "order" | "ordering" => match value {
                        "acc" | "a" | "accuracy" => sort_by = Some(ScoreOrder::Acc),
                        "combo" | "c" => sort_by = Some(ScoreOrder::Combo),
                        "date" | "d" | "recent" | "r" => sort_by = Some(ScoreOrder::Date),
                        "length" | "len" | "l" => sort_by = Some(ScoreOrder::Length),
                        "pp" | "p" => sort_by = Some(ScoreOrder::Pp),
                        _ => {
                            let content = "Failed to parse `sort`.\n\
                            Must be either `acc`, `combo`, `date`, `length`, or `pp`";

                            return Err(content.into());
                        }
                    },
                    "mods" => match matcher::get_mods(value) {
                        Some(mods_) => mods = Some(mods_),
                        None => return Err(Self::ERR_PARSE_MODS.into()),
                    },
                    "reverse" | "r" => match value {
                        "true" | "t" | "1" => reverse = Some(true),
                        "false" | "f" | "0" => reverse = Some(false),
                        _ => {
                            let content =
                                "Failed to parse `reverse`. Must be either `true` or `false`.";

                            return Err(content.into());
                        }
                    },
                    _ => {
                        let content = format!(
                            "Unrecognized option `{key}`.\n\
                            Available options are: `acc`, `combo`, `sort`, `grade`, or `reverse`."
                        );

                        return Err(content.into());
                    }
                }
            } else if let Some(mods_) = matcher::get_mods(arg.as_ref()) {
                mods = Some(mods_);
            } else {
                match matcher::get_mention_user(arg.as_ref()) {
                    Some(id) => discord = Some(id),
                    None => name = Some(arg),
                }
            }
        }

        let args = Self {
            name,
            discord,
            mode,
            mods,
            min_acc: acc_min,
            max_acc: acc_max,
            min_combo: combo_min,
            max_combo: combo_max,
            grade,
            sort_by: sort_by.unwrap_or_default().into(),
            reverse: reverse.unwrap_or(false),
            perfect_combo: None,
            index: num.map(|n| n as usize),
            query: None,
            farm: None,
            size: None,
            has_dash_r: has_dash_r.unwrap_or(false),
            has_dash_p_or_i: has_dash_p_or_i.unwrap_or(false),
        };

        Ok(args)
    }
}

impl TryFrom<Top> for TopArgs<'static> {
    type Error = &'static str;

    fn try_from(args: Top) -> Result<Self, Self::Error> {
        let mods = match args.mods() {
            ModsResult::Mods(mods) => Some(mods),
            ModsResult::None => None,
            ModsResult::Invalid => return Err(Self::ERR_PARSE_MODS),
        };

        Ok(Self {
            name: args.name.map(Cow::Owned),
            discord: args.discord,
            mode: args.mode.map(GameMode::from),
            mods,
            min_acc: None,
            max_acc: None,
            min_combo: None,
            max_combo: None,
            grade: args.grade.map(Grade::from),
            sort_by: args.sort.unwrap_or_default(),
            reverse: args.reverse.unwrap_or(false),
            perfect_combo: args.perfect_combo,
            index: args.index.map(|n| n as usize),
            query: args.query,
            farm: args.farm,
            size: args.size,
            has_dash_r: false,
            has_dash_p_or_i: false,
        })
    }
}

const FARM_CUTOFF: usize = 727;

pub(super) async fn top(
    ctx: Arc<Context>,
    orig: CommandOrigin<'_>,
    args: TopArgs<'_>,
) -> Result<()> {
    if args.index.filter(|n| *n > 100).is_some() {
        let content = "Can't have more than 100 top scores.";

        return orig.error(&ctx, content).await;
    }

    let mut config = match ctx.user_config().with_osu_id(orig.user_id()?).await {
        Ok(config) => config,
        Err(err) => {
            let _ = orig.error(&ctx, GENERAL_ISSUE).await;

            return Err(err.wrap_err("failed to get user config"));
        }
    };

    let mode = args.mode.or(config.mode).unwrap_or(GameMode::Osu);

    if args.sort_by == TopScoreOrder::Pp && args.has_dash_r {
        let mode_long = mode_long(mode);
        let prefix = ctx.guild_config().first_prefix(orig.guild_id()).await;

        let mode_short = match mode {
            GameMode::Osu => "",
            GameMode::Mania => "m",
            GameMode::Taiko => "t",
            GameMode::Catch => "c",
        };

        let content = format!(
            "`{prefix}top{mode_long} -r`? I think you meant `{prefix}recentbest{mode_long}` \
            or `{prefix}rb{mode_short}` for short ;)",
        );

        return orig.error(&ctx, content).await;
    } else if args.has_dash_p_or_i {
        let cmd = match args.sort_by {
            TopScoreOrder::Date => "rb",
            TopScoreOrder::Pp => "top",
            _ => unreachable!(),
        };

        let mode_long = mode_long(mode);
        let prefix = ctx.guild_config().first_prefix(orig.guild_id()).await;

        let content = format!(
            "`{prefix}{cmd}{mode_long} -i / -p`? \
            Try putting the number right after the command, e.g. \
            `{prefix}{cmd}{mode_long}42`, or use the pagination buttons.",
        );

        return orig.error(&ctx, content).await;
    }

    let user_id = match user_id!(ctx, orig, args) {
        Some(user_id) => user_id,
        None => match config.osu.take() {
            Some(user_id) => UserId::Id(user_id),
            None => return require_link(&ctx, &orig).await,
        },
    };

    // Retrieve the user and their top scores
    let user_args = UserArgs::rosu_id(&ctx, &user_id).await.mode(mode);
    let scores_fut = ctx.osu_scores().top().limit(100).exec_with_user(user_args);

    let farm_fut = async {
        if args.farm.is_some() || matches!(args.sort_by, TopScoreOrder::Farm) {
            let stats = match ctx.redis().osutracker_stats().await {
                Ok(stats) => stats,
                Err(err) => return Some(Err(err)),
            };

            let farm = match stats {
                RedisData::Original(stats) => stats
                    .mapset_count
                    .into_iter()
                    .enumerate()
                    .map(|(i, entry): (_, OsuTrackerMapsetEntry)| {
                        (entry.mapset_id, (entry, i < FARM_CUTOFF))
                    })
                    .collect::<Farm>(),
                RedisData::Archived(stats) => stats
                    .mapset_count
                    .iter()
                    .map(|entry| entry.deserialize(&mut Infallible).unwrap())
                    .enumerate()
                    .map(|(i, entry): (_, OsuTrackerMapsetEntry)| {
                        (entry.mapset_id, (entry, i < FARM_CUTOFF))
                    })
                    .collect::<Farm>(),
            };

            Some(Ok(farm))
        } else {
            None
        }
    };

    let (user_score_res, farm_res) = tokio::join!(scores_fut, farm_fut);

    let (user, scores) = match user_score_res {
        Ok((user, scores)) => (user, scores),
        Err(OsuError::NotFound) => {
            let content = user_not_found(&ctx, user_id).await;

            return orig.error(&ctx, content).await;
        }
        Err(err) => {
            let _ = orig.error(&ctx, OSU_API_ISSUE).await;
            let err = Report::new(err).wrap_err("failed to get user or scores");

            return Err(err);
        }
    };

    let farm = match farm_res {
        Some(Ok(mapsets)) => mapsets,
        Some(Err(err)) => {
            let _ = orig.error(&ctx, OSUTRACKER_ISSUE).await;

            return Err(err.wrap_err("failed to get farm"));
        }
        None => HashMap::default(),
    };

    // Filter scores according to mods, combo, acc, and grade
    let entries = match process_scores(&ctx, scores, &args, &farm).await {
        Ok(entries) => entries,
        Err(err) => {
            let _ = orig.error(&ctx, GENERAL_ISSUE).await;

            return Err(err.wrap_err("failed to process scores"));
        }
    };

    let username = user.username();

    if args.index.filter(|n| *n > entries.len()).is_some() {
        let content = format!(
            "`{username}` only has {} top scores with the specified properties",
            entries.len(),
        );

        return orig.error(&ctx, content).await;
    }

    let (guild_score_size, guild_list_size, guild_minimized_pp) = match orig.guild_id() {
        Some(guild_id) => {
            let f =
                |config: &GuildConfig| (config.score_size, config.list_size, config.minimized_pp);

            ctx.guild_config().peek(guild_id, f).await
        }
        None => (None, None, None),
    };

    match (args.index, entries.len()) {
        (Some(num), _) => {
            let score_size = config.score_size.or(guild_score_size).unwrap_or_default();

            let minimized_pp = config
                .minimized_pp
                .or(guild_minimized_pp)
                .unwrap_or_default();

            let entry = &entries[num.saturating_sub(1)];

            single_embed(ctx, orig, user, entry, score_size, minimized_pp, None).await?;
        }
        (_, 1) => {
            let score_size = config.score_size.or(guild_score_size).unwrap_or_default();

            let minimized_pp = config
                .minimized_pp
                .or(guild_minimized_pp)
                .unwrap_or_default();

            let content = write_content(username, &args, 1);
            let entry = &entries[0];

            single_embed(ctx, orig, user, entry, score_size, minimized_pp, content).await?;
        }
        (None, _) => {
            let content = write_content(username, &args, entries.len());

            let list_size = args
                .size
                .or(config.list_size)
                .or(guild_list_size)
                .unwrap_or_default();

            match list_size {
                ListSize::Condensed => {
                    TopCondensedPagination::builder(user, entries, args.sort_by, farm)
                        .content(content.unwrap_or_default())
                        .start_by_update()
                        .defer_components()
                        .start(ctx, orig)
                        .await?;
                }
                ListSize::Detailed => {
                    TopPagination::builder(user, entries, args.sort_by, farm)
                        .content(content.unwrap_or_default())
                        .start_by_update()
                        .defer_components()
                        .start(ctx, orig)
                        .await?;
                }
                ListSize::Single => {
                    let minimized_pp = config
                        .minimized_pp
                        .or(guild_minimized_pp)
                        .unwrap_or_default();

                    TopSinglePagination::builder(user, entries, minimized_pp)
                        .content(content.unwrap_or_default())
                        .start_by_update()
                        .defer_components()
                        .start(ctx, orig)
                        .await?;
                }
            }
        }
    }

    Ok(())
}

pub struct TopEntry {
    pub original_idx: usize,
    pub score: ScoreSlim,
    pub map: OsuMap,
    pub max_pp: f32,
    pub stars: f32,
}

async fn process_scores(
    ctx: &Context,
    scores: Vec<Score>,
    args: &TopArgs<'_>,
    farm: &Farm,
) -> Result<Vec<TopEntry>> {
    let mut entries = Vec::with_capacity(scores.len());

    let acc_range = match (args.min_acc, args.max_acc) {
        (None, None) => None,
        (None, Some(max)) => Some(0.0..=max),
        (Some(min), None) => Some(min..=100.0),
        (Some(min), Some(max)) => Some(min..=max),
    };

    let combo_range = match (args.min_combo, args.max_combo) {
        (None, None) => None,
        (None, Some(max)) => Some(0..=max),
        (Some(min), None) => Some(min..=u32::MAX),
        (Some(min), Some(max)) => Some(min..=max),
    };

    let filter_criteria = args.query.as_deref().map(FilterCriteria::new);

    let maps_id_checksum = scores
        .iter()
        .filter(|score| match acc_range {
            Some(ref range) => range.contains(&score.accuracy),
            None => true,
        })
        .filter(|score| match combo_range {
            Some(ref range) => range.contains(&score.max_combo),
            None => true,
        })
        .filter(|score| match args.grade {
            Some(grade) => score.grade.eq_letter(grade),
            None => true,
        })
        .filter(|score| match args.mods {
            None => true,
            Some(ModSelection::Include(mods @ GameMods::NoMod) | ModSelection::Exact(mods)) => {
                score.mods == mods
            }
            Some(ModSelection::Include(mods)) => score.mods.contains(mods),
            Some(ModSelection::Exclude(GameMods::NoMod)) => !score.mods.is_empty(),
            Some(ModSelection::Exclude(mods)) => !score.mods.intersects(mods),
        })
        .filter(|score| match args.farm {
            None => true,
            Some(FarmFilter::OnlyFarm) => {
                let mapset_id = score
                    .map
                    .as_ref()
                    .map(|map| map.mapset_id)
                    .or_else(|| score.mapset.as_ref().map(|mapset| mapset.mapset_id))
                    .expect("neither map nor mapset available");

                farm.get(&mapset_id).map_or(false, |(_, farm)| *farm)
            }
            Some(FarmFilter::NoFarm) => {
                let mapset_id = score
                    .map
                    .as_ref()
                    .map(|map| map.mapset_id)
                    .or_else(|| score.mapset.as_ref().map(|mapset| mapset.mapset_id))
                    .expect("neither map nor mapset available");

                farm.get(&mapset_id).map_or(true, |(_, farm)| !*farm)
            }
        })
        .filter(|score| match filter_criteria {
            Some(ref criteria) => score.matches(criteria),
            None => true,
        })
        .filter(|score| {
            match score
                .map
                .as_ref()
                .and_then(|map| map.max_combo)
                .zip(args.perfect_combo)
            {
                Some((max_combo, perfect_combo)) => perfect_combo == (max_combo == score.max_combo),
                None => true,
            }
        })
        .map(|score| {
            (
                score.map_id as i32,
                score.map.as_ref().and_then(|map| map.checksum.as_deref()),
            )
        })
        .collect();

    let mut maps = ctx.osu_map().maps(&maps_id_checksum).await?;

    for (i, score) in scores.into_iter().enumerate() {
        let Some(map) = maps.remove(&score.map_id) else { continue };

        let attrs = ctx
            .pp(&map)
            .mode(score.mode)
            .mods(score.mods)
            .performance()
            .await;

        let pp = score.pp.expect("missing pp");

        let max_pp = if score.grade.eq_letter(Grade::X) && score.mode != GameMode::Mania {
            pp
        } else {
            attrs.pp() as f32
        };

        let entry = TopEntry {
            original_idx: i,
            score: ScoreSlim::new(score, pp),
            map,
            max_pp,
            stars: attrs.stars() as f32,
        };

        entries.push(entry);
    }

    match args.sort_by {
        TopScoreOrder::Acc => entries.sort_by(|a, b| {
            b.score
                .accuracy
                .partial_cmp(&a.score.accuracy)
                .unwrap_or(Ordering::Equal)
        }),
        TopScoreOrder::Bpm => entries.sort_by(|a, b| {
            let a_bpm = a.map.bpm() * a.score.mods.clock_rate();
            let b_bpm = b.map.bpm() * b.score.mods.clock_rate();

            b_bpm.partial_cmp(&a_bpm).unwrap_or(Ordering::Equal)
        }),
        TopScoreOrder::Combo => entries.sort_by_key(|entry| Reverse(entry.score.max_combo)),
        TopScoreOrder::Date => entries.sort_by_key(|entry| Reverse(entry.score.ended_at)),
        TopScoreOrder::Farm => entries.sort_by(|a, b| {
            let mapset_a = a.map.mapset_id();
            let mapset_b = b.map.mapset_id();

            let count_a = farm.get(&mapset_a).map_or(0, |(entry, _)| entry.count);
            let count_b = farm.get(&mapset_b).map_or(0, |(entry, _)| entry.count);

            count_b.cmp(&count_a)
        }),
        TopScoreOrder::Length => {
            entries.sort_by(|a, b| {
                let a_len = a.map.seconds_drain() as f32 / a.score.mods.clock_rate();
                let b_len = b.map.seconds_drain() as f32 / b.score.mods.clock_rate();

                b_len.partial_cmp(&a_len).unwrap_or(Ordering::Equal)
            });
        }
        TopScoreOrder::Misses => entries.sort_by(|a, b| {
            b.score
                .statistics
                .count_miss
                .cmp(&a.score.statistics.count_miss)
                .then_with(|| {
                    let hits_a = a.score.total_hits();
                    let hits_b = b.score.total_hits();

                    let ratio_a = a.score.statistics.count_miss as f32 / hits_a as f32;
                    let ratio_b = b.score.statistics.count_miss as f32 / hits_b as f32;

                    ratio_b
                        .partial_cmp(&ratio_a)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| hits_b.cmp(&hits_a))
                })
        }),
        TopScoreOrder::Pp => entries.sort_by(|a, b| {
            b.score
                .pp
                .partial_cmp(&a.score.pp)
                .unwrap_or(Ordering::Equal)
        }),
        TopScoreOrder::RankedDate => entries.sort_by_key(|entry| Reverse(entry.map.ranked_date())),
        TopScoreOrder::Score => entries.sort_by_key(|entry| Reverse(entry.score.score)),
        TopScoreOrder::Stars => {
            entries.sort_by(|a, b| b.stars.partial_cmp(&a.stars).unwrap_or(Ordering::Equal))
        }
    }

    if args.reverse {
        entries.reverse();
    }

    Ok(entries)
}

fn mode_long(mode: GameMode) -> &'static str {
    match mode {
        GameMode::Osu => "",
        GameMode::Mania => "mania",
        GameMode::Taiko => "taiko",
        GameMode::Catch => "ctb",
    }
}

#[allow(clippy::too_many_arguments)]
async fn single_embed(
    ctx: Arc<Context>,
    orig: CommandOrigin<'_>,
    user: RedisData<User>,
    entry: &TopEntry,
    score_size: ScoreSize,
    minimized_pp: MinimizedPp,
    content: Option<String>,
) -> Result<()> {
    // Prepare retrieval of the map's global top 50 and the user's top 100
    let global_idx = match entry.map.status() {
        Ranked | Loved | Qualified | Approved => {
            // TODO: Add .limit(50) when supported by osu!api
            match ctx.osu().beatmap_scores(entry.map.map_id()).await {
                Ok(scores) => {
                    let user_id = user.user_id();

                    scores.iter().position(|s| {
                        s.user_id == user_id
                            && (entry.score.ended_at.unix_timestamp() - s.ended_at.unix_timestamp())
                                .abs()
                                <= 2
                    })
                }
                Err(err) => {
                    let err = Report::new(err).wrap_err("failed to get global scores");
                    warn!("{err:?}");

                    None
                }
            }
        }
        _ => None,
    };

    let personal_idx = Some(entry.original_idx);
    let embed_fut = TopSingleEmbed::new(&user, entry, personal_idx, global_idx, minimized_pp, &ctx);

    let embed = embed_fut.await;

    // Only maximize if config allows it
    match score_size {
        ScoreSize::AlwaysMinimized => {
            let mut builder = MessageBuilder::new().embed(embed.into_minimized());

            if let Some(content) = content {
                builder = builder.content(content);
            }

            orig.create_message(&ctx, &builder).await?;
        }
        ScoreSize::InitialMaximized => {
            let mut builder = MessageBuilder::new().embed(embed.as_maximized());

            if let Some(ref content) = content {
                builder = builder.content(content);
            }

            let response = orig.create_message(&ctx, &builder).await?.model().await?;

            ctx.store_msg(response.id);

            // Minimize embed after delay
            tokio::spawn(async move {
                sleep(Duration::from_secs(45)).await;

                if !ctx.remove_msg(response.id) {
                    return;
                }

                let mut builder = MessageBuilder::new().embed(embed.into_minimized());

                if let Some(content) = content {
                    builder = builder.content(content);
                }

                if let Err(err) = response.update(&ctx, &builder).await {
                    let report = Report::new(err).wrap_err("Failed to minimize embed");
                    warn!("{report:?}");
                }
            });
        }
        ScoreSize::AlwaysMaximized => {
            let mut builder = MessageBuilder::new().embed(embed.as_maximized());

            if let Some(content) = content {
                builder = builder.content(content);
            }

            orig.create_message(&ctx, &builder).await?;
        }
    }

    Ok(())
}

type Farm = HashMap<u32, (OsuTrackerMapsetEntry, bool), IntHasher>;

fn write_content(name: &str, args: &TopArgs<'_>, amount: usize) -> Option<String> {
    let condition = args.min_acc.is_some()
        || args.max_acc.is_some()
        || args.min_combo.is_some()
        || args.max_combo.is_some()
        || args.grade.is_some()
        || args.mods.is_some()
        || args.perfect_combo.is_some()
        || args.query.is_some()
        || args.farm.is_some();

    if condition {
        Some(content_with_condition(args, amount))
    } else {
        let genitive = if name.ends_with('s') { "" } else { "s" };
        let reverse = if args.reverse { "reversed " } else { "" };

        let content = match args.sort_by {
            TopScoreOrder::Farm if args.reverse => {
                format!("`{name}`'{genitive} top100 sorted by least popular farm:")
            }
            TopScoreOrder::Farm => {
                format!("`{name}`'{genitive} top100 sorted by most popular farm:")
            }
            TopScoreOrder::Acc => {
                format!("`{name}`'{genitive} top100 sorted by {reverse}accuracy:")
            }
            TopScoreOrder::Bpm => {
                format!("`{name}`'{genitive} top100 sorted by {reverse}BPM:")
            }
            TopScoreOrder::Combo => {
                format!("`{name}`'{genitive} top100 sorted by {reverse}combo:")
            }
            TopScoreOrder::Date if args.reverse => {
                format!("Oldest scores in `{name}`'{genitive} top100:")
            }
            TopScoreOrder::Date => {
                format!("Most recent scores in `{name}`'{genitive} top100:")
            }
            TopScoreOrder::Length => {
                format!("`{name}`'{genitive} top100 sorted by {reverse}length:")
            }
            TopScoreOrder::Misses => {
                format!("`{name}`'{genitive} top100 sorted by {reverse}miss count:")
            }
            TopScoreOrder::Pp if !args.reverse => return None,
            TopScoreOrder::Pp => {
                format!("`{name}`'{genitive} top100 sorted by reversed pp:")
            }
            TopScoreOrder::RankedDate => {
                format!("`{name}`'{genitive} top100 sorted by {reverse}ranked date:")
            }
            TopScoreOrder::Score => {
                format!("`{name}`'{genitive} top100 sorted by {reverse}score:")
            }
            TopScoreOrder::Stars => {
                format!("`{name}`'{genitive} top100 sorted by {reverse}stars:")
            }
        };

        Some(content)
    }
}

fn content_with_condition(args: &TopArgs<'_>, amount: usize) -> String {
    let mut content = String::with_capacity(64);

    match args.sort_by {
        TopScoreOrder::Farm => content.push_str("`Order: Farm"),
        TopScoreOrder::Acc => content.push_str("`Order: Accuracy"),
        TopScoreOrder::Bpm => content.push_str("`Order: BPM"),
        TopScoreOrder::Combo => content.push_str("`Order: Combo"),
        TopScoreOrder::Date => content.push_str("`Order: Date"),
        TopScoreOrder::Length => content.push_str("`Order: Length"),
        TopScoreOrder::Misses => content.push_str("`Order: Miss count"),
        TopScoreOrder::Pp => content.push_str("`Order: Pp"),
        TopScoreOrder::RankedDate => content.push_str("`Order: Ranked date"),
        TopScoreOrder::Score => content.push_str("`Order: Score"),
        TopScoreOrder::Stars => content.push_str("`Order: Stars"),
    }

    if args.reverse {
        content.push_str(" (reverse)`");
    } else {
        content.push('`');
    }

    match (args.min_acc, args.max_acc) {
        (None, None) => {}
        (None, Some(max)) => {
            let _ = write!(content, " ~ `Acc: 0% - {}%`", round(max));
        }
        (Some(min), None) => {
            let _ = write!(content, " ~ `Acc: {}% - 100%`", round(min));
        }
        (Some(min), Some(max)) => {
            let _ = write!(content, " ~ `Acc: {}% - {}%`", round(min), round(max));
        }
    }

    match (args.min_combo, args.max_combo) {
        (None, None) => {}
        (None, Some(max)) => {
            let _ = write!(content, " ~ `Combo: 0 - {max}`");
        }
        (Some(min), None) => {
            let _ = write!(content, " ~ `Combo: {min} - ∞`");
        }
        (Some(min), Some(max)) => {
            let _ = write!(content, " ~ `Combo: {min} - {max}`");
        }
    }

    if let Some(grade) = args.grade {
        let _ = write!(content, " ~ `Grade: {grade}`");
    }

    if let Some(selection) = args.mods {
        let (pre, mods) = match selection {
            ModSelection::Include(mods) => ("Include ", mods),
            ModSelection::Exclude(mods) => ("Exclude ", mods),
            ModSelection::Exact(mods) => ("", mods),
        };

        let _ = write!(content, " ~ `Mods: {pre}{mods}`");
    }

    if let Some(perfect_combo) = args.perfect_combo {
        let _ = write!(content, " ~ `Perfect combo: {perfect_combo}`");
    }

    if let Some(query) = args.query.as_deref() {
        let _ = write!(content, " ~ `Query: {query}`");
    }

    match args.farm {
        Some(FarmFilter::OnlyFarm) => content.push_str(" ~ `Only farm`"),
        Some(FarmFilter::NoFarm) => content.push_str(" ~ `Without farm`"),
        None => {}
    }

    let plural = if amount == 1 { "" } else { "s" };
    let _ = write!(content, "\nFound {amount} matching top score{plural}:");

    content
}
