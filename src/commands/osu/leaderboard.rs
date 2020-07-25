use crate::{
    arguments::{Args, MapModArgs, ModSelection},
    embeds::{EmbedData, LeaderboardEmbed},
    pagination::{LeaderboardPagination, Pagination},
    util::{
        constants::{AVATAR_URL, GENERAL_ISSUE, OSU_API_ISSUE},
        discord, MessageExt,
    },
    BotResult, Context,
};

use rosu::{
    backend::requests::BeatmapRequest,
    models::{
        ApprovalStatus::{Approved, Loved, Ranked},
        GameMods,
    },
};
use std::sync::Arc;
use twilight::model::channel::Message;

#[allow(clippy::cognitive_complexity)]
async fn leaderboard_send(
    national: bool,
    ctx: Arc<Context>,
    msg: &Message,
    args: Args,
) -> BotResult<()> {
    let author_name = {
        let data = ctx.data.read().await;
        data.get::<DiscordLinks>()
            .and_then(|links| links.get(msg.author.id.as_u64()).cloned())
    };
    let args = MapModArgs::new(args);
    let map_id = if let Some(id) = args.map_id {
        id.get()
    } else {
        let msgs = msg
            .channel_id
            .messages(ctx, |retriever| retriever.limit(50))
            .await?;
        match discord::map_id_from_history(msgs, &ctx.cache).await {
            Some(id) => id,
            None => {
                let content = "No beatmap specified and none found in recent channel history. \
                        Try specifying a map either by url to the map, or just by map id.";
                msg.respond(&ctx, content).await?;
                return Ok(());
            }
        }
    };
    let (mods, selection) = args
        .mods
        .unwrap_or_else(|| (GameMods::default(), ModSelection::None));

    // Retrieving the beatmap
    let (map_to_db, map) = {
        let data = ctx.data.read().await;
        let mysql = data.get::<MySQL>().unwrap();
        match mysql.get_beatmap(map_id).await {
            Ok(map) => (false, map),
            Err(_) => {
                let map_req = BeatmapRequest::new().map_id(map_id);
                let osu = data.get::<Osu>().unwrap();
                let map = match map_req.queue_single(&osu).await {
                    Ok(result) => match result {
                        Some(map) => map,
                        None => {
                            let content = format!(
                                "Could not find beatmap with id `{}`. \
                                Did you give me a mapset id instead of a map id?",
                                map_id
                            );
                            msg.respond(&ctx, content).await?;
                            return Ok(());
                        }
                    },
                    Err(why) => {
                        msg.respond(ctx, OSU_API_ISSUE).await?;
                        return Err(why.into());
                    }
                };
                (
                    map.approval_status == Ranked
                        || map.approval_status == Loved
                        || map.approval_status == Approved,
                    map,
                )
            }
        }
    };

    // Retrieve the map's leaderboard
    let scores = {
        let data = ctx.data.read().await;
        let scraper = data.get::<Scraper>().unwrap();
        let scores_future = scraper.get_leaderboard(
            map_id,
            national,
            match selection {
                ModSelection::Excludes | ModSelection::None => None,
                _ => Some(&mods),
            },
        );
        match scores_future.await {
            Ok(scores) => scores,
            Err(why) => {
                msg.respond(&ctx.http, OSU_API_ISSUE).await?;
                return Err(why.into());
            }
        }
    };
    let amount = scores.len();

    // Accumulate all necessary data
    let map_copy = if map_to_db { Some(map.clone()) } else { None };
    let first_place_icon = scores
        .first()
        .map(|s| format!("{}{}", AVATAR_URL, s.user_id));
    let data = match LeaderboardEmbed::new(
        &author_name.as_deref(),
        &map,
        if scores.is_empty() {
            None
        } else {
            Some(scores.iter().take(10))
        },
        &first_place_icon,
        0,
        ctx,
    )
    .await
    {
        Ok(data) => data,
        Err(why) => {
            msg.respond(&ctx, GENERAL_ISSUE).await?;
            return Err(why);
        }
    };

    // Sending the embed
    let response = msg
        .channel_id
        .send_message(&ctx.http, |m| {
            let content = format!(
                "I found {} scores with the specified mods on the map's leaderboard",
                amount
            );
            m.content(content).embed(|e| data.build(e))
        })
        .await;

    // Add map to database if its not in already
    if let Some(map) = map_copy {
        let data = ctx.data.read().await;
        let mysql = data.get::<MySQL>().unwrap();
        if let Err(why) = mysql.insert_beatmap(&map).await {
            warn!("Could not add map of recent command to DB: {}", why);
        }
    }
    let resp = response?;
    if scores.is_empty() {
        resp.reaction_delete(ctx, msg.author.id).await;
        return Ok(());
    }

    // Skip pagination if too few entries
    if scores.len() <= 10 {
        resp.reaction_delete(ctx, msg.author.id).await;
        return Ok(());
    }

    // Pagination
    let pagination = LeaderboardPagination::new(
        ctx,
        resp,
        msg.author.id,
        map,
        scores,
        author_name,
        first_place_icon,
    )
    .await;
    let cache = Arc::clone(&ctx.cache);
    let http = Arc::clone(&ctx.http);
    tokio::spawn(async move {
        if let Err(why) = pagination.start(cache, http).await {
            warn!("Pagination error: {}", why)
        }
    });
    Ok(())
}

#[command]
#[short_desc("Display the belgian leaderboard of a map")]
#[long_desc(
    "Display the belgian leaderboard of a given map. \
     If no map is given, I will choose the last map \
     I can find in my embeds of this channel"
)]
#[usage("[map url / map id]")]
#[example("2240404")]
#[example("https://osu.ppy.sh/beatmapsets/902425#osu/2240404")]
#[aliases("lb")]
pub async fn leaderboard(ctx: Arc<Context>, msg: &Message) -> BotResult<()> {
    leaderboard_send(true, ctx, msg, args).await
}

#[command]
#[short_desc("Display the global leaderboard of a map")]
#[long_desc(
    "Display the global leaderboard of a given map. \
     If no map is given, I will choose the last map \
     I can find in my embeds of this channel"
)]
#[usage("[map url / map id]")]
#[example("2240404")]
#[example("https://osu.ppy.sh/beatmapsets/902425#osu/2240404")]
#[aliases("glb")]
pub async fn globalleaderboard(ctx: Arc<Context>, msg: &Message) -> BotResult<()> {
    leaderboard_send(false, ctx, msg, args).await
}
