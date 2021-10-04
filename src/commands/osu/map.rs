use crate::{
    bail,
    commands::{
        osu::{option_map, option_mods},
        MyCommand,
    },
    embeds::{EmbedData, MapEmbed},
    pagination::{MapPagination, Pagination},
    util::{
        constants::{
            common_literals::{MAP, MAP_PARSE_FAIL, MODS, MODS_PARSE_FAIL},
            GENERAL_ISSUE, OSU_API_ISSUE,
        },
        error::PPError,
        matcher,
        osu::{
            map_id_from_history, map_id_from_msg, prepare_beatmap_file, MapIdType, ModSelection,
        },
        ApplicationCommandExt, MessageExt,
    },
    Args, BotResult, CommandData, Context, Error, MessageBuilder,
};

use chrono::Duration;
use image::{png::PngEncoder, ColorType, DynamicImage, GenericImage, GenericImageView, Pixel};
use plotters::prelude::*;
use rosu_pp::{Beatmap, BeatmapExt};
use rosu_v2::prelude::{GameMode, GameMods, OsuError};
use std::{cmp::Ordering, sync::Arc};
use tokio::fs::File;
use twilight_model::{
    application::interaction::{application_command::CommandDataOption, ApplicationCommand},
    channel::message::MessageType,
};

const W: u32 = 590;
const H: u32 = 150;

#[command]
#[short_desc("Display a bunch of stats about a map(set)")]
#[long_desc(
    "Display stats about a beatmap. Mods can be specified.\n\
    If no map(set) is specified by either url or id, I will choose the last map \
    I can find in the embeds of this channel.\n\
    If the mapset is specified by id but there is some map with the same id, \
    I will choose the latter."
)]
#[usage("[map(set) url / map(set) id] [+mods]")]
#[example("2240404 +hddt", "https://osu.ppy.sh/beatmapsets/902425 +hr")]
#[aliases("m", "beatmap", "maps", "beatmaps", "mapinfo")]
async fn map(ctx: Arc<Context>, data: CommandData) -> BotResult<()> {
    match data {
        CommandData::Message { msg, mut args, num } => match MapArgs::args(&mut args) {
            Ok(mut map_args) => {
                let reply = msg
                    .referenced_message
                    .as_ref()
                    .filter(|_| msg.kind == MessageType::Reply);

                if let Some(id) = reply.and_then(|msg| map_id_from_msg(msg)) {
                    map_args.map = Some(id);
                }

                _map(ctx, CommandData::Message { msg, args, num }, map_args).await
            }
            Err(content) => msg.error(&ctx, content).await,
        },
        CommandData::Interaction { command } => slash_map(ctx, *command).await,
    }
}

async fn _map(ctx: Arc<Context>, data: CommandData<'_>, args: MapArgs) -> BotResult<()> {
    let MapArgs { map, mods } = args;
    let author_id = data.author()?.id;

    let map_id = if let Some(id) = map {
        id
    } else {
        let msgs = match ctx.retrieve_channel_history(data.channel_id()).await {
            Ok(msgs) => msgs,
            Err(why) => {
                let _ = data.error(&ctx, GENERAL_ISSUE).await;

                return Err(why);
            }
        };

        match map_id_from_history(&msgs) {
            Some(id) => id,
            None => {
                let content = "No beatmap specified and none found in recent channel history. \
                    Try specifying a map(set) either by url to the map, \
                    or just by map(set) id.";

                return data.error(&ctx, content).await;
            }
        }
    };

    let mods = match mods {
        Some(selection) => selection.mods(),
        None => GameMods::NoMod,
    };

    // Retrieving the beatmaps
    let (mapset_id, map_id) = match map_id {
        // If its given as map id, try to convert into mapset id
        MapIdType::Map(id) => {
            // Check if map is in DB
            match ctx.psql().get_beatmap(id, false).await {
                Ok(map) => (map.mapset_id, Some(id)),
                Err(_) => {
                    // If not in DB, request through API
                    match ctx.osu().beatmap().map_id(id).await {
                        Ok(map) => (map.mapset_id, Some(id)),
                        Err(OsuError::NotFound) => (id, None),
                        Err(why) => {
                            let _ = data.error(&ctx, OSU_API_ISSUE).await;

                            return Err(why.into());
                        }
                    }
                }
            }
        }

        // If its already given as mapset id, do nothing
        MapIdType::Set(id) => (id, None),
    };

    // Request mapset through API
    let (mapset, maps) = match ctx.osu().beatmapset(mapset_id).await {
        Ok(mut mapset) => {
            let mut maps = mapset.maps.take().unwrap_or_default();

            maps.sort_unstable_by(|m1, m2| {
                (m1.mode as u8)
                    .cmp(&(m2.mode as u8))
                    .then_with(|| match m1.mode {
                        // For mania sort first by mania key, then star rating
                        GameMode::MNA => m1
                            .cs
                            .partial_cmp(&m2.cs)
                            .unwrap_or(Ordering::Equal)
                            .then(m1.stars.partial_cmp(&m2.stars).unwrap_or(Ordering::Equal)),
                        // For other mods just sort by star rating
                        _ => m1.stars.partial_cmp(&m2.stars).unwrap_or(Ordering::Equal),
                    })
            });

            (mapset, maps)
        }
        Err(OsuError::NotFound) => {
            let content = format!("Could find neither map nor mapset with id {}", mapset_id);

            return data.error(&ctx, content).await;
        }
        Err(why) => {
            let _ = data.error(&ctx, OSU_API_ISSUE).await;

            return Err(why.into());
        }
    };

    let map_count = maps.len();

    let map_idx = if maps.is_empty() {
        return data.error(&ctx, "The mapset has no maps").await;
    } else {
        map_id
            .and_then(|map_id| maps.iter().position(|map| map.map_id == map_id))
            .unwrap_or(0)
    };

    let map = &maps[map_idx];

    // Try creating the strain graph for the map
    let bg_fut = async {
        let url = mapset.covers.cover.as_str();
        let res = reqwest::get(url).await?.bytes().await?;

        Ok::<_, Error>(image::load_from_memory(res.as_ref())?.thumbnail_exact(W, H))
    };

    let graph = match tokio::join!(strain_values(map.map_id, mods), bg_fut) {
        (Ok(strain_values), Ok(img)) => match graph(strain_values, img) {
            Ok(graph) => Some(graph),
            Err(why) => {
                unwind_error!(warn, why, "Error creating graph: {}");

                None
            }
        },
        (Err(why), _) => {
            unwind_error!(warn, why, "Error while creating oppai_values: {}");

            None
        }
        (_, Err(why)) => {
            unwind_error!(warn, why, "Error retrieving graph background: {}");

            None
        }
    };

    // Accumulate all necessary data
    let data_fut = MapEmbed::new(
        map,
        &mapset,
        mods,
        graph.is_none(),
        (map_idx + 1, map_count),
    );

    let embed_data = match data_fut.await {
        Ok(data) => data,
        Err(why) => {
            let _ = data.error(&ctx, GENERAL_ISSUE).await;

            return Err(why);
        }
    };

    // Sending the embed
    let embed = embed_data.into_builder().build();
    let mut builder = MessageBuilder::new().embed(embed);

    if let Some(bytes) = graph.as_deref() {
        builder = builder.file("map_graph.png", bytes);
    }

    let response_raw = data.create_message(&ctx, builder).await?;

    // Add mapset and maps to database
    let (mapset_result, maps_result) = tokio::join!(
        ctx.clients.psql.insert_beatmapset(&mapset),
        ctx.clients.psql.insert_beatmaps(&maps),
    );

    if let Err(why) = mapset_result {
        unwind_error!(warn, why, "Error while adding mapset to DB: {}");
    }

    if let Err(why) = maps_result {
        unwind_error!(warn, why, "Error while adding maps to DB: {}");
    }

    // Skip pagination if too few entries
    if map_count == 1 {
        return Ok(());
    }

    let response = response_raw.model().await?;

    // Pagination
    let pagination = MapPagination::new(response, mapset, maps, mods, map_idx, graph.is_none());
    let owner = author_id;

    tokio::spawn(async move {
        if let Err(why) = pagination.start(&ctx, owner, 60).await {
            unwind_error!(warn, why, "Pagination error (map): {}")
        }
    });

    Ok(())
}

async fn strain_values(map_id: u32, mods: GameMods) -> BotResult<Vec<(f32, f32)>> {
    let map_path = prepare_beatmap_file(map_id).await?;
    let file = File::open(map_path).await.map_err(PPError::from)?;
    let map = Beatmap::parse(file).await.map_err(PPError::from)?;
    let strains = map.strains(mods.bits());
    let section_len = strains.section_length;

    let strains = strains
        .strains
        .into_iter()
        .scan(0.0, |time, strain| {
            *time += section_len;

            Some((*time, strain))
        })
        .collect();

    Ok(strains)
}

fn graph(strains: Vec<(f32, f32)>, mut background: DynamicImage) -> BotResult<Vec<u8>> {
    static LEN: usize = W as usize * H as usize;

    let max_strain = strains
        .iter()
        .copied()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(Ordering::Equal))
        .map_or(0.0, |(_, s)| s);

    if max_strain <= std::f32::EPSILON {
        bail!("no non-zero strain point");
    }

    let mut buf = vec![0; LEN * 3]; // PIXEL_SIZE = 3

    {
        let root = BitMapBackend::with_buffer(&mut buf, (W, H)).into_drawing_area();
        root.fill(&WHITE)?;
        let mut chart = ChartBuilder::on(&root)
            .x_label_area_size(17)
            .build_cartesian_2d(0.0..strains.last().unwrap().0, 0.0..max_strain)?;

        // Make background darker and sum up rgb values to find minimum
        let (width, height) = background.dimensions();
        let mut r = 0;
        let mut g = 0;
        let mut b = 0;

        for y in 0..height {
            for x in 0..width {
                let pixel = background
                    .get_pixel(x, y)
                    .map_with_alpha(|c| c.saturating_sub(75), |a| a.saturating_sub(25));

                r += pixel[0] as u64;
                g += pixel[1] as u64;
                b += pixel[2] as u64;

                background.put_pixel(x, y, pixel);
            }
        }

        // Take as line color whatever is represented least in the background
        let b = (b as f32 * 1.3) as u64;
        let line_color = match r.min(g).min(b) {
            min if min == r => &RED,
            min if min == g => &GREEN,
            min if min == b => &BLUE,
            _ => unreachable!(),
        };

        // Add background
        let elem: BitMapElement<_> = ((0.0, max_strain), background).into();
        chart.draw_series(std::iter::once(elem))?;

        // Mesh and labels
        let text_style = FontDesc::new(FontFamily::Serif, 12.0, FontStyle::Bold).color(line_color);
        chart
            .configure_mesh()
            .disable_y_mesh()
            .disable_y_axis()
            .set_all_tick_mark_size(3)
            .light_line_style(&BLACK.mix(0.0))
            .x_labels(10)
            .x_label_style(text_style)
            .x_label_formatter(&|timestamp| {
                if timestamp.abs() < f32::EPSILON {
                    return String::new();
                }

                let d = Duration::milliseconds(*timestamp as i64);
                let minutes = d.num_seconds() / 60;
                let seconds = d.num_seconds() % 60;

                format!("{}:{:0>2}", minutes, seconds)
            })
            .draw()?;

        // Draw line
        chart.draw_series(LineSeries::new(
            strains.into_iter().map(|(time, strain)| (time, strain)),
            line_color,
        ))?;
    }

    // Encode buf to png
    let mut png_bytes: Vec<u8> = Vec::with_capacity(LEN);
    let png_encoder = PngEncoder::new(&mut png_bytes);
    png_encoder.encode(&buf, W, H, ColorType::Rgb8)?;

    Ok(png_bytes)
}

trait TupleExt: Sized {
    fn add(self, other: (u64, u64, u64)) -> Self;
}

impl TupleExt for (u64, u64, u64) {
    fn add(self, other: (u64, u64, u64)) -> Self {
        (self.0 + other.0, self.1 + other.1, self.2 + other.2)
    }
}

struct MapArgs {
    map: Option<MapIdType>,
    mods: Option<ModSelection>,
}

impl MapArgs {
    fn args(args: &mut Args) -> Result<Self, String> {
        let mut map = None;
        let mut mods = None;

        for arg in args.take(2) {
            if let Some(id) =
                matcher::get_osu_map_id(arg).or_else(|| matcher::get_osu_mapset_id(arg))
            {
                map = Some(id);
            } else if let Some(mods_) = matcher::get_mods(arg) {
                mods = Some(mods_);
            } else {
                let content = format!(
                    "Failed to parse `{}`.\n\
                    Be sure you specify either a valid map id, map url, or mod combination.",
                    arg
                );

                return Err(content);
            }
        }

        Ok(Self { map, mods })
    }

    fn slash(command: &mut ApplicationCommand) -> BotResult<Result<Self, &'static str>> {
        let mut map = None;
        let mut mods = None;

        for option in command.yoink_options() {
            match option {
                CommandDataOption::String { name, value } => match name.as_str() {
                    MAP => match matcher::get_osu_map_id(&value)
                        .or_else(|| matcher::get_osu_mapset_id(&value))
                    {
                        Some(id) => map = Some(id),
                        None => return Ok(Err(MAP_PARSE_FAIL)),
                    },
                    MODS => match matcher::get_mods(&value) {
                        Some(mods_) => mods = Some(mods_),
                        None => match value.parse() {
                            Ok(mods_) => mods = Some(ModSelection::Exact(mods_)),
                            Err(_) => return Ok(Err(MODS_PARSE_FAIL)),
                        },
                    },
                    _ => bail_cmd_option!(MAP, string, name),
                },
                CommandDataOption::Integer { name, .. } => {
                    bail_cmd_option!(MAP, integer, name)
                }
                CommandDataOption::Boolean { name, .. } => {
                    bail_cmd_option!(MAP, boolean, name)
                }
                CommandDataOption::SubCommand { name, .. } => {
                    bail_cmd_option!(MAP, subcommand, name)
                }
            }
        }

        Ok(Ok(Self { map, mods }))
    }
}

pub async fn slash_map(ctx: Arc<Context>, mut command: ApplicationCommand) -> BotResult<()> {
    match MapArgs::slash(&mut command)? {
        Ok(args) => _map(ctx, command.into(), args).await,
        Err(content) => command.error(&ctx, content).await,
    }
}

pub fn define_map() -> MyCommand {
    let map = option_map();
    let mods = option_mods(false);

    let help = "Display a bunch of stats about a map(set).\n\
        The values in the map info will be adjusted to mods.\n\
        Since discord does not allow images to be adjusted when editing messages, \
        the strain graph always belongs to the initial map, even after moving to \
        other maps of the set through the arrow reactions.";

    MyCommand::new(MAP, "Display a bunch of stats about map(set)")
        .help(help)
        .options(vec![map, mods])
}
