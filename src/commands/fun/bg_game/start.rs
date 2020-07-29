use super::ReactionWrapper;
use crate::{
    bail,
    bg_game::MapsetTags,
    database::MapsetTagWrapper,
    embeds::{BGStartEmbed, BGTagsEmbed, EmbedData},
    util::{constants::GENERAL_ISSUE, MessageExt},
    Args, BotResult, Context,
};

use rosu::models::GameMode;
use std::sync::Arc;
use tokio::{stream::StreamExt, time::Duration};
use twilight::model::{
    channel::{Message, Reaction, ReactionType},
    gateway::{
        event::{Event, EventType},
        payload::{ReactionAdd, ReactionRemove},
    },
};

#[command]
#[bucket("bg_start")]
#[short_desc("Start the bg game or skip the current background")]
#[aliases("s", "resolve", "r", "skip")]
pub async fn start(ctx: Arc<Context>, msg: &Message, mut args: Args) -> BotResult<()> {
    let mode = match args.next() {
        Some("m") | Some("mania") => GameMode::MNA,
        _ => GameMode::STD,
    };
    let mapsets = match get_mapsets(&ctx, msg, mode).await {
        Ok(mapsets) => mapsets,
        Err(why) => {
            let _ = msg.error(&ctx, GENERAL_ISSUE).await;
            bail!("Error while getting mapsets: {}", why);
        }
    };
    if !mapsets.is_empty() {
        ctx.add_game_and_start(ctx.clone(), msg.channel_id, mapsets);
    }
    Ok(())
}

async fn get_mapsets(
    ctx: &Context,
    msg: &Message,
    mode: GameMode,
) -> BotResult<Vec<MapsetTagWrapper>> {
    if mode == GameMode::MNA {
        return ctx.psql().get_all_tags_mapset(GameMode::MNA).await;
    }
    // Send initial message
    let data = BGStartEmbed::new(msg.author.id);
    let embed = data.build().build();
    let response = ctx
        .http
        .create_message(msg.channel_id)
        .embed(embed)?
        .await?;

    // Prepare the reaction stream
    let self_id = ctx.cache.bot_user.id;
    let reaction_add_stream = ctx
        .standby
        .wait_for_reaction_stream(response.id, move |event: &ReactionAdd| {
            event.user_id != self_id
        })
        .filter_map(|reaction: ReactionAdd| Some(ReactionWrapper::Add(reaction.0)));
    let reaction_remove_stream = ctx
        .standby
        .wait_for_event_stream(EventType::ReactionRemove, |_: &Event| true)
        .filter_map(|event: Event| {
            if let Event::ReactionRemove(reaction) = event {
                if reaction.0.message_id == response.id && reaction.0.user_id != self_id {
                    return Some(ReactionWrapper::Remove(reaction.0));
                }
            }
            None
        });
    let mut reaction_stream = reaction_add_stream
        .merge(reaction_remove_stream)
        .timeout(Duration::from_secs(60));

    // Send initial reactions
    let reactions = [
        "🍋",
        "🤓",
        "🤡",
        "🎨",
        "🍨",
        "👨‍🌾",
        "😱",
        "🪀",
        "🟦",
        "🗽",
        "🌀",
        "👴",
        "💯",
        "✅",
        "❌",
    ];
    for &reaction in reactions.iter() {
        let emote = ReactionType::Unicode {
            name: reaction.to_string(),
        };
        ctx.http
            .create_reaction(response.channel_id, response.id, emote)
            .await?;
    }
    let mut included = MapsetTags::empty();
    let mut excluded = MapsetTags::empty();

    // Start collecting
    while let Some(Ok(reaction)) = reaction_stream.next().await {
        let tag = if let ReactionType::Unicode { ref name } = reaction.as_deref().emoji {
            match name.as_str() {
                "🍋" => MapsetTags::Easy,
                "🤓" => MapsetTags::Hard,
                "🤡" => MapsetTags::Meme,
                "👴" => MapsetTags::Old,
                "😱" => MapsetTags::HardName,
                "🟦" => MapsetTags::BlueSky,
                "🪀" => MapsetTags::Alternate,
                "🗽" => MapsetTags::English,
                "👨‍🌾" => MapsetTags::Farm,
                "💯" => MapsetTags::Tech,
                "🎨" => MapsetTags::Weeb,
                "🌀" => MapsetTags::Streams,
                "🍨" => MapsetTags::Kpop,
                "✅" if reaction.as_deref().user_id == msg.author.id => break,
                "❌" if reaction.as_deref().user_id == msg.author.id => {
                    msg.reply(ctx, "Game cancelled").await?;
                    return Ok(Vec::new());
                }
                _ => continue,
            }
        } else {
            continue;
        };
        match reaction {
            ReactionWrapper::Add(_) => {
                included.insert(tag);
                excluded.remove(tag);
            }
            ReactionWrapper::Remove(_) => {
                excluded.insert(tag);
                included.remove(tag);
            }
        }
    }
    for &reaction in reactions.iter() {
        let r = ReactionType::Unicode {
            name: reaction.to_string(),
        };
        if msg.guild_id.is_none() {
            ctx.http
                .delete_current_user_reaction(msg.channel_id, msg.id, r)
                .await?;
        } else {
            ctx.http
                .delete_all_reaction(msg.channel_id, msg.id, r)
                .await?;
        }
    }
    // Get all mapsets matching the given tags
    debug_assert_eq!(mode, GameMode::STD);
    let mapsets = match ctx
        .psql()
        .get_specific_tags_mapset(mode, included, excluded)
        .await
    {
        Ok(mapsets) => mapsets,
        Err(why) => {
            let _ = msg.error(ctx, GENERAL_ISSUE).await;
            bail!("Error while getting specific tags: {}", why);
        }
    };
    let data = BGTagsEmbed::new(included, excluded, mapsets.len());
    let embed = data.build().build();
    ctx.http
        .create_message(msg.channel_id)
        .embed(embed)?
        .await?;
    if !mapsets.is_empty() {
        debug!(
            "Starting bg game with included: {} - excluded: {}",
            included.join(','),
            excluded.join(',')
        );
    }
    Ok(mapsets)
}
