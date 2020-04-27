use crate::{
    embeds::{BasicEmbedData, RecentData},
    scraper::{MostPlayedMap, ScraperScore},
    util::numbers,
    Error, Osu,
};

use rosu::models::{Beatmap, GameMode, Score, User};
use serenity::{
    builder::CreateEmbed,
    cache::CacheRwLock,
    prelude::{RwLock, ShareMap},
};
use std::{collections::HashMap, sync::Arc};

pub enum PaginationType {
    MostPlayed {
        user: Box<User>,
        maps: Vec<MostPlayedMap>,
    },
    Recent {
        user: Box<User>,
        scores: Vec<Score>,
        maps: HashMap<u32, Beatmap>,
        best: Vec<Score>,
        global: HashMap<u32, Vec<Score>>,
        cache: CacheRwLock,
        data: Arc<RwLock<ShareMap>>,
    },
    Top {
        user: Box<User>,
        scores: Vec<(usize, Score, Beatmap)>,
        mode: GameMode,
        cache: CacheRwLock,
        data: Arc<RwLock<ShareMap>>,
    },
    Leaderboard {
        map: Box<Beatmap>,
        scores: Vec<ScraperScore>,
        author_name: Option<String>,
        first_place_icon: Option<String>,
        cache: CacheRwLock,
        data: Arc<RwLock<ShareMap>>,
    },
}

/// Page data created upon reactions
pub enum ReactionData {
    Delete,
    None,
    Basic(Box<BasicEmbedData>),
    Recent(Box<RecentData>),
}

impl ReactionData {
    pub fn build(self, embed: &mut CreateEmbed) -> &mut CreateEmbed {
        match self {
            ReactionData::Basic(data) => data.build(embed),
            ReactionData::Recent(data) => data.build(embed),
            _ => panic!("Don't call ReactionData::build on non-Basic/Recent"),
        }
    }

    pub fn recent_data(self) -> RecentData {
        match self {
            ReactionData::Recent(data) => *data,
            _ => panic!("Don't call ReactionData::recent_data on anything but Recent"),
        }
    }
}

impl From<RecentData> for ReactionData {
    fn from(data: RecentData) -> Self {
        Self::Recent(Box::new(data))
    }
}

pub struct Pagination {
    pub index: usize,
    last_index: usize,
    per_page: usize,
    total_pages: usize,
    pagination: PaginationType,
}

impl Pagination {
    pub fn most_played(user: User, maps: Vec<MostPlayedMap>) -> Self {
        let amount = maps.len();
        let per_page = 10;
        let pagination = PaginationType::MostPlayed {
            user: Box::new(user),
            maps,
        };
        Self {
            index: 0,
            per_page,
            total_pages: numbers::div_euclid(per_page, amount),
            last_index: last_multiple(per_page, amount),
            pagination,
        }
    }

    pub fn recent(
        user: User,
        scores: Vec<Score>,
        maps: HashMap<u32, Beatmap>,
        best: Vec<Score>,
        global: HashMap<u32, Vec<Score>>,
        cache: CacheRwLock,
        data: Arc<RwLock<ShareMap>>,
    ) -> Self {
        let total_pages = scores.len();
        let pagination = PaginationType::Recent {
            user: Box::new(user),
            scores,
            maps,
            best,
            global,
            cache,
            data,
        };
        Self {
            index: 0,
            per_page: 5,
            total_pages,
            last_index: total_pages - 1,
            pagination,
        }
    }

    pub fn top(
        user: User,
        scores: Vec<(usize, Score, Beatmap)>,
        mode: GameMode,
        cache: CacheRwLock,
        data: Arc<RwLock<ShareMap>>,
    ) -> Self {
        let amount = scores.len();
        let per_page = 5;
        let pagination = PaginationType::Top {
            user: Box::new(user),
            scores,
            mode,
            cache,
            data,
        };
        Self {
            index: 0,
            per_page,
            total_pages: numbers::div_euclid(per_page, amount),
            last_index: last_multiple(per_page, amount),
            pagination,
        }
    }

    pub fn leaderboard(
        map: Beatmap,
        scores: Vec<ScraperScore>,
        author_name: Option<String>,
        first_place_icon: Option<String>,
        cache: CacheRwLock,
        data: Arc<RwLock<ShareMap>>,
    ) -> Self {
        let amount = scores.len();
        let per_page = 10;
        let pagination = PaginationType::Leaderboard {
            map: Box::new(map),
            scores,
            author_name,
            first_place_icon,
            cache,
            data,
        };
        Self {
            index: 0,
            per_page,
            total_pages: numbers::div_euclid(per_page, amount),
            last_index: last_multiple(per_page, amount),
            pagination,
        }
    }

    pub async fn next_reaction(&mut self, reaction: &str) -> Result<ReactionData, Error> {
        let next_index = match reaction {
            // Move to start
            "⏮️" => {
                if self.index > 0 {
                    Some(0)
                } else {
                    None
                }
            }
            // Move one page left
            "⏪" => self.index.checked_sub(self.per_page),
            // Move one index left
            "◀️" => self.index.checked_sub(1),
            // Move one index right
            "▶️" => {
                if self.index < self.last_index {
                    Some(self.index + 1)
                } else {
                    None
                }
            }
            // Move one page right
            "⏩" => {
                let index = self.index + self.per_page;
                if index <= self.last_index {
                    Some(index)
                } else {
                    None
                }
            }
            // Move to end
            "⏭️" => {
                if self.index < self.last_index {
                    Some(self.last_index)
                } else {
                    None
                }
            }
            "❌" => return Ok(ReactionData::Delete),
            _ => None,
        };
        if let Some(next_index) = next_index {
            self.index = next_index;
            self.page_data().await
        } else {
            Ok(ReactionData::None)
        }
    }

    #[allow(clippy::map_entry)]
    async fn page_data(&mut self) -> Result<ReactionData, Error> {
        let page = self.index / self.per_page + 1;
        let result = match &mut self.pagination {
            // Most Played
            PaginationType::MostPlayed { user, maps } => {
                ReactionData::Basic(Box::new(BasicEmbedData::create_mostplayed(
                    user,
                    maps.iter().skip(self.index).take(self.per_page),
                    (page, self.total_pages),
                )))
            }
            // Recent
            PaginationType::Recent {
                user,
                scores,
                maps,
                best,
                global,
                cache,
                data,
            } => {
                let score = scores.get(self.index).unwrap();
                let map_id = score.beatmap_id.unwrap();
                // Make sure map is ready
                if !maps.contains_key(&map_id) {
                    let data = data.read().await;
                    let osu = data.get::<Osu>().expect("Could not get osu client");
                    let map = score.get_beatmap(osu).await?;
                    maps.insert(map_id, map);
                }
                let map = maps.get(&map_id).unwrap();
                // Make sure map leaderboard is ready
                if !global.contains_key(&map.beatmap_id) {
                    let data = data.read().await;
                    let osu = data.get::<Osu>().expect("Could not get Osu");
                    let global_lb = map.get_global_leaderboard(&osu, 50).await?;
                    global.insert(map.beatmap_id, global_lb);
                };
                let global_lb = global.get(&map.beatmap_id).unwrap();
                // Create embed data
                ReactionData::Recent(Box::new(
                    RecentData::new(user, score, map, best, global_lb, (cache, data)).await?,
                ))
            }
            // Top / Recent Best / ...
            PaginationType::Top {
                user,
                scores,
                mode,
                cache,
                data,
            } => ReactionData::Basic(Box::new(
                BasicEmbedData::create_top(
                    user,
                    scores.iter().skip(self.index).take(5),
                    *mode,
                    (page, self.total_pages),
                    (cache, data),
                )
                .await?,
            )),
            // Leaderboard
            PaginationType::Leaderboard {
                map,
                scores,
                author_name,
                first_place_icon,
                cache,
                data,
            } => ReactionData::Basic(Box::new(
                BasicEmbedData::create_leaderboard(
                    &author_name.as_deref(),
                    map,
                    Some(scores.iter().skip(self.index).take(10)),
                    first_place_icon,
                    self.index,
                    (cache, data),
                )
                .await?,
            )),
        };
        Ok(result)
    }

    pub fn recent_maps(self) -> HashMap<u32, Beatmap> {
        match self.pagination {
            PaginationType::Recent { maps, .. } => maps,
            _ => panic!("Don't call Pagination::recent_maps on anything but Recent"),
        }
    }
}

fn last_multiple(per_page: usize, total: usize) -> usize {
    if total % per_page == 0 {
        total - per_page
    } else {
        total - total % per_page
    }
}