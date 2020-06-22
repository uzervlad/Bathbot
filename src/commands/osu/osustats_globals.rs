use crate::{
    arguments::OsuStatsArgs,
    embeds::{EmbedData, OsuStatsGlobalsEmbed},
    scraper::{OsuStatsScore, Scraper},
    util::{globals::OSU_API_ISSUE, MessageExt},
    DiscordLinks, Osu,
};

use rosu::{backend::requests::UserRequest, models::GameMode};
use serenity::{
    framework::standard::{macros::command, Args, CommandResult},
    model::prelude::Message,
    prelude::Context,
};
use std::collections::BTreeMap;

async fn osustats_send(mode: GameMode, ctx: &Context, msg: &Message, args: Args) -> CommandResult {
    let name = {
        let data = ctx.data.read().await;
        let links = data.get::<DiscordLinks>().unwrap();
        links.get(msg.author.id.as_u64()).cloned()
    };
    let args = match OsuStatsArgs::new(args, name, mode) {
        Ok(args) => args,
        Err(err_msg) => {
            msg.channel_id
                .say(ctx, err_msg)
                .await?
                .reaction_delete(ctx, msg.author.id)
                .await;
            return Ok(());
        }
    };
    let params = args.params;
    let user = {
        let req = UserRequest::with_username(&params.username).mode(mode);
        let data = ctx.data.read().await;
        let osu = data.get::<Osu>().unwrap();
        match req.queue_single(osu).await {
            Ok(Some(user)) => user,
            Ok(None) => {
                msg.channel_id
                    .say(ctx, format!("User `{}` was not found", params.username))
                    .await?
                    .reaction_delete(ctx, msg.author.id)
                    .await;
                return Ok(());
            }
            Err(why) => {
                msg.channel_id
                    .say(ctx, OSU_API_ISSUE)
                    .await?
                    .reaction_delete(ctx, msg.author.id)
                    .await;
                return Err(why.to_string().into());
            }
        }
    };
    let scores: BTreeMap<usize, OsuStatsScore> = {
        let data = ctx.data.read().await;
        let scraper = data.get::<Scraper>().unwrap();
        match scraper.get_global_scores(&params).await {
            Ok(scores) => scores.into_iter().enumerate().collect(),
            Err(why) => {
                msg.channel_id
                    .say(ctx, OSU_API_ISSUE)
                    .await?
                    .reaction_delete(ctx, msg.author.id)
                    .await;
                return Err(why.to_string().into());
            }
        }
    };

    // Accumulate all necessary data
    let data = match OsuStatsGlobalsEmbed::new(&user, &scores, 0, ctx).await {
        Ok(data) => data,
        Err(why) => {
            msg.channel_id
                .say(
                    ctx,
                    "Some issue while calculating osustatsglobals data, blame bade",
                )
                .await?
                .reaction_delete(ctx, msg.author.id)
                .await;
            return Err(why.to_string().into());
        }
    };

    // Creating the embed
    msg.channel_id
        .send_message(ctx, |m| m.embed(|e| data.build(e)))
        .await?
        .reaction_delete(ctx, msg.author.id)
        .await;
    Ok(())
}

#[command]
#[description = "Show all scores of a player that are on a map's global leaderboard.\n\
Rank and accuracy range can be specified with `-r` and `-a`. \
After this keyword, you must specify either a number for max rank/acc, \
or two numbers of the form `a..b` for min and max rank/acc.\n\
There are several available orderings: Accuracy with `--a`, combo with `--c`, \
pp with `--p`, rank with `--r`, score with `--s`, misses with `--m`, \
and the default: date.\n\
By default the scores are sorted in descending order. To reverse, specify `--asc`.\n
Mods can also be specified."]
#[usage = "[username] [mods] [-a [num..]num] [-r [num..]num] [--a/--c/--p/--r/--s/--m] [--asc]"]
#[example = "badewanne3 -dt! -a 97.5..99.5 -r 42 --p --asc"]
#[example = "vaxei +hdhr -r 1..5 --r"]
#[aliases("osg")]
pub async fn osustatsglobals(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
    osustats_send(GameMode::STD, ctx, msg, args).await
}

#[command]
#[description = "Show all scores of a player that are on a mania map's global leaderboard.\n\
Rank and accuracy range can be specified with `-r` and `-a`. \
After this keyword, you must specify either a number for max rank/acc, \
or two numbers of the form `a..b` for min and max rank/acc.\n\
There are several available orderings: Accuracy with `--a`, combo with `--c`, \
pp with `--p`, rank with `--r`, score with `--s`, misses with `--m`, \
and the default: date.\n\
By default the scores are sorted in descending order. To reverse, specify `--asc`.\n
Mods can also be specified."]
#[usage = "[username] [mods] [-a [num..]num] [-r [num..]num] [--a/--c/--p/--r/--s/--m] [--asc]"]
#[example = "badewanne3 -dt! -a 97.5..99.5 -r 42 --p --asc"]
#[example = "vaxei +hdhr -r 1..5 --r"]
#[aliases("osgm")]
pub async fn osustatsglobalsmania(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
    osustats_send(GameMode::MNA, ctx, msg, args).await
}

#[command]
#[description = "Show all scores of a player that are on a taiko map's global leaderboard.\n\
Rank and accuracy range can be specified with `-r` and `-a`. \
After this keyword, you must specify either a number for max rank/acc, \
or two numbers of the form `a..b` for min and max rank/acc.\n\
There are several available orderings: Accuracy with `--a`, combo with `--c`, \
pp with `--p`, rank with `--r`, score with `--s`, misses with `--m`, \
and the default: date.\n\
By default the scores are sorted in descending order. To reverse, specify `--asc`.\n
Mods can also be specified."]
#[usage = "[username] [mods] [-a [num..]num] [-r [num..]num] [--a/--c/--p/--r/--s/--m] [--asc]"]
#[example = "badewanne3 -dt! -a 97.5..99.5 -r 42 --p --asc"]
#[example = "vaxei +dtmr -r 1..5 --r"]
#[aliases("osgt")]
pub async fn osustatsglobalstaiko(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
    osustats_send(GameMode::TKO, ctx, msg, args).await
}

#[command]
#[description = "Show all scores of a player that are on a ctb map's global leaderboard.\n\
Rank and accuracy range can be specified with `-r` and `-a`. \
After this keyword, you must specify either a number for max rank/acc, \
or two numbers of the form `a..b` for min and max rank/acc.\n\
There are several available orderings: Accuracy with `--a`, combo with `--c`, \
pp with `--p`, rank with `--r`, score with `--s`, misses with `--m`, \
and the default: date.\n\
By default the scores are sorted in descending order. To reverse, specify `--asc`.\n
Mods can also be specified."]
#[usage = "[username] [mods] [-a [num..]num] [-r [num..]num] [--a/--c/--p/--r/--s/--m] [--asc]"]
#[example = "badewanne3 -dt! -a 97.5..99.5 -r 42 --p --asc"]
#[example = "vaxei +hdhr -r 1..5 --r"]
#[aliases("osgc")]
pub async fn osustatsglobalsctb(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
    osustats_send(GameMode::CTB, ctx, msg, args).await
}
