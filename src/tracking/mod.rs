mod tracking_loop;

pub use tracking_loop::{process_tracking, tracking_loop};

use crate::{database::TrackingUser, BotResult, Database};

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use priority_queue::PriorityQueue;
use rosu::models::GameMode;
use std::{cmp::Reverse, collections::HashMap, iter};
use tokio::{sync::RwLock, time};
use twilight::model::id::ChannelId;

lazy_static::lazy_static! {
    pub static ref OSU_TRACKING_INTERVAL: Duration = Duration::seconds(3600);
    pub static ref OSU_TRACKING_COOLDOWN: Duration = Duration::seconds(5);
}

type TrackingQueue = RwLock<PriorityQueue<(u32, GameMode), Reverse<DateTime<Utc>>>>;

pub struct OsuTracking {
    queue: TrackingQueue,
    users: DashMap<(u32, GameMode), TrackingUser>,
    last_date: RwLock<DateTime<Utc>>,
    pub interval: RwLock<Duration>,
    pub cooldown: RwLock<Duration>,
}

impl OsuTracking {
    pub async fn new(psql: &Database) -> BotResult<Self> {
        let users = psql.get_osu_trackings().await?;
        let queue = users
            .iter()
            .map(|guard| {
                let value = guard.value();
                ((value.user_id, value.mode), Reverse(Utc::now()))
            })
            .collect();
        Ok(Self {
            queue: RwLock::new(queue),
            users,
            last_date: RwLock::new(Utc::now()),
            interval: RwLock::new(*OSU_TRACKING_INTERVAL),
            cooldown: RwLock::new(*OSU_TRACKING_COOLDOWN),
        })
    }

    pub async fn reset(&self, user: u32, mode: GameMode) {
        let mut queue = self.queue.write().await;
        let now = Utc::now();
        *self.last_date.write().await = now;
        queue.push_decrease((user, mode), Reverse(now));
    }

    pub async fn update_last_date(
        &self,
        user_id: u32,
        mode: GameMode,
        new_date: DateTime<Utc>,
        psql: &Database,
    ) -> BotResult<()> {
        if let Some(mut tracked_user) = self.users.get_mut(&(user_id, mode)) {
            tracked_user.last_top_score = new_date;
            psql.update_osu_tracking(user_id, mode, new_date, &tracked_user.channels)
                .await?;
        }
        Ok(())
    }

    pub fn get_tracked(
        &self,
        user_id: u32,
        mode: GameMode,
    ) -> Option<(DateTime<Utc>, HashMap<ChannelId, usize>)> {
        self.users
            .get(&(user_id, mode))
            .map(|user| (user.last_top_score, user.channels.to_owned()))
    }

    pub async fn pop(&self) -> Option<HashMap<(u32, GameMode), DateTime<Utc>>> {
        let len = self.queue.read().await.len();
        debug!(
            "[Popping] Amount: {} ~ Last pop: {:?}",
            len,
            *self.last_date.read().await
        );
        if len == 0 {
            return None;
        }
        // Calculate how many users need to be popped for this iteration
        // so that _all_ users will be popped within the next INTERVAL
        let interval = *self.last_date.read().await + *self.interval.read().await - Utc::now();
        let ms_per_track = interval.num_milliseconds() as f32 / len as f32;
        let amount = (self.cooldown.read().await.num_milliseconds() as f32 / ms_per_track).max(1.0);
        let delay = (ms_per_track * amount) as u64;
        time::delay_for(time::Duration::from_millis(delay)).await;
        debug!(
            "Waited {}ms ~ ms_per_track: {} ~ Popping {}",
            delay, ms_per_track, amount
        );
        // Pop users and return them
        let elems = {
            let mut queue = self.queue.write().await;
            iter::repeat_with(|| queue.pop().map(|(key, _)| key))
                .take(amount as usize)
                .flatten()
                .map(|key| {
                    let last_top_score = self.users.get(&key).unwrap().last_top_score;
                    ((key.0, key.1), last_top_score)
                })
                .collect()
        };
        *self.last_date.write().await = Utc::now();
        Some(elems)
    }

    pub async fn remove_user(
        &self,
        user_id: u32,
        channel: ChannelId,
        psql: &Database,
    ) -> BotResult<()> {
        let removed: Vec<_> = self
            .users
            .iter_mut()
            .filter(|guard| guard.key().0 == user_id)
            .filter_map(
                |mut guard| match guard.value_mut().remove_channel(channel) {
                    true => Some(guard.key().1),
                    false => None,
                },
            )
            .collect();
        for mode in removed {
            let key = (user_id, mode);
            match self
                .users
                .get(&key)
                .map(|guard| guard.value().channels.is_empty())
            {
                Some(true) => {
                    debug!("Removing ({},{}) from tracking", user_id, mode);
                    psql.remove_osu_tracking(user_id, mode).await?;
                    println!("removed");
                    self.queue.write().await.remove(&key);
                    println!("wrote");
                    self.users.remove(&key);
                    println!("done");
                }
                Some(false) => {
                    let guard = self.users.get(&key).unwrap();
                    let user = guard.value();
                    psql.update_osu_tracking(user_id, mode, user.last_top_score, &user.channels)
                        .await?
                }
                None => warn!("Should not be reachable"),
            }
        }
        Ok(())
    }

    pub async fn remove_channel(
        &self,
        channel: ChannelId,
        mode: Option<GameMode>,
        psql: &Database,
    ) -> BotResult<usize> {
        let iter = self.users.iter_mut().filter(|guard| match mode {
            Some(mode) => guard.key().1 == mode,
            None => true,
        });
        let mut count = 0;
        for mut guard in iter {
            if !guard.value_mut().remove_channel(channel) {
                continue;
            }
            let key = guard.key();
            let tracked_user = guard.value();
            if tracked_user.channels.is_empty() {
                debug!("Removing {:?} from tracking (all)", key);
                psql.remove_osu_tracking(key.0, key.1).await?;
                self.queue.write().await.remove(&key);
                self.users.remove(&key);
            } else {
                psql.update_osu_tracking(
                    key.0,
                    key.1,
                    tracked_user.last_top_score,
                    &tracked_user.channels,
                )
                .await?
            }
            count += 1;
        }
        Ok(count)
    }

    pub async fn add(
        &self,
        user_id: u32,
        mode: GameMode,
        last_top_score: DateTime<Utc>,
        channel: ChannelId,
        limit: usize,
        psql: &Database,
    ) -> BotResult<bool> {
        let key = (user_id, mode);
        match self.users.get_mut(&key) {
            Some(mut guard) => match guard.value().channels.get(&channel) {
                Some(old_limit) => match *old_limit == limit {
                    true => return Ok(false),
                    false => {
                        let value = guard.value_mut();
                        value.channels.insert(channel, limit);
                        psql.update_osu_tracking(
                            user_id,
                            mode,
                            value.last_top_score,
                            &value.channels,
                        )
                        .await?;
                    }
                },
                None => {
                    let value = guard.value_mut();
                    value.channels.insert(channel, limit);
                    psql.update_osu_tracking(user_id, mode, value.last_top_score, &value.channels)
                        .await?;
                }
            },
            None => {
                debug!("Inserting {:?} for tracking", key);
                psql.insert_osu_tracking(user_id, mode, last_top_score, channel, limit)
                    .await?;
                let tracking_user =
                    TrackingUser::new(user_id, mode, last_top_score, channel, limit);
                self.users.insert(key, tracking_user);
                let now = Utc::now();
                *self.last_date.write().await = now;
                let mut queue = self.queue.write().await;
                queue.push((user_id, mode), Reverse(now));
            }
        }
        Ok(true)
    }

    pub fn list(&self, channel: ChannelId) -> Vec<(u32, GameMode, usize)> {
        self.users
            .iter()
            .filter_map(|guard| {
                let limit = *guard.value().channels.get(&channel)?;
                let (user_id, mode) = guard.key();
                Some((*user_id, *mode, limit))
            })
            .collect()
    }
}
