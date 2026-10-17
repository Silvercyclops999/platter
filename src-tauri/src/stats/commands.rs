//! Tauri commands for listening stats (docs/IPC.md). Each one first counts
//! any days that changed since the last call, then reads.

use tauri::State;

use super::compare::{self, CompareResult, CompareSummary};
use super::throwback::{self, OnThisDay, TimeTravel};
use super::wrapped::{self, TopSongsIndex, TopSongsList, Wrapped};
use super::{ItemKind, ItemStats, Moments, MonthRecap, Point, StatsOverview, StatsRange, StatsSpan, TopKind};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};

/// Count the days that changed, when any did.
pub async fn fresh(db: &DbHandle) -> AppResult<()> {
    if db.read_async(super::needs_refresh).await? {
        db.write_async(|c| super::refresh(c).map(|_| ())).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn stats_overview(db: State<'_, DbHandle>, range: StatsRange, limit: Option<u32>) -> AppResult<StatsOverview> {
    fresh(&db).await?;
    db.read_async(move |c| {
        let d = super::resolve(c, &range)?;
        super::overview(c, d, limit.unwrap_or(50))
    })
    .await
}

/// Small stories from a range (first song, most replayed day, biggest
/// discovery, a song you came back to).
#[tauri::command]
pub async fn stats_moments(db: State<'_, DbHandle>, range: StatsRange) -> AppResult<Moments> {
    fresh(&db).await?;
    db.read_async(move |c| {
        let d = super::resolve(c, &range)?;
        super::moments(c, d)
    })
    .await
}

/// Each day with plays from `from` to `to` (the calendar).
#[tauri::command]
pub async fn stats_days(db: State<'_, DbHandle>, from: String, to: String) -> AppResult<Vec<Point>> {
    fresh(&db).await?;
    db.read_async(move |c| {
        let (lo, hi) = (super::parse_day(&from)?, super::parse_day(&to)?);
        if hi < lo || hi - lo > super::DAYS_MAX {
            return Err(AppError::Invalid("That calendar range is too long.".into()));
        }
        super::days(c, lo, hi)
    })
    .await
}

#[tauri::command]
pub async fn stats_span(db: State<'_, DbHandle>) -> AppResult<StatsSpan> {
    fresh(&db).await?;
    db.read_async(super::span).await
}

#[tauri::command]
pub async fn stats_item(db: State<'_, DbHandle>, kind: ItemKind, id: String) -> AppResult<ItemStats> {
    fresh(&db).await?;
    db.read_async(move |c| {
        let today = super::today(c)?;
        super::item(c, kind, &id, today)
    })
    .await
}

#[tauri::command]
pub async fn stats_month_recap(db: State<'_, DbHandle>, month: String) -> AppResult<MonthRecap> {
    fresh(&db).await?;
    db.read_async(move |c| super::month_recap(c, &month)).await
}

/// Songs for a top list's "Play these" and "Save as playlist".
#[tauri::command]
pub async fn stats_play_ids(db: State<'_, DbHandle>, range: StatsRange, kind: TopKind, limit: Option<u32>) -> AppResult<Vec<String>> {
    fresh(&db).await?;
    db.read_async(move |c| {
        let d = super::resolve(c, &range)?;
        super::play_ids(c, d, kind, limit.unwrap_or(50))
    })
    .await
}

/// One answer kept for the rest of its day, until the counted plays change.
type DayCache<T> = parking_lot::Mutex<Option<(i64, throwback::Stamp, T)>>;
static ON_THIS_DAY: DayCache<OnThisDay> = parking_lot::const_mutex(None);
static TIME_TRAVEL: DayCache<TimeTravel> = parking_lot::const_mutex(None);

/// Today and what the counted plays look like, for the day caches.
async fn day_and_stamp(db: &DbHandle) -> AppResult<(i64, throwback::Stamp)> {
    fresh(db).await?;
    db.read_async(|c| Ok((super::today(c)?, throwback::stamp(c)?))).await
}

fn cached<T: Clone>(cache: &DayCache<T>, today: i64, stamp: throwback::Stamp) -> Option<T> {
    cache.lock().as_ref().filter(|(d, s, _)| *d == today && *s == stamp).map(|(_, _, v)| v.clone())
}

/// Home's On this day shelf (empty with `shown: false` while it's off).
#[tauri::command]
pub async fn home_on_this_day(db: State<'_, DbHandle>) -> AppResult<OnThisDay> {
    if !crate::settings::load(&db).home_on_this_day {
        return Ok(OnThisDay::default());
    }
    let (today, stamp) = day_and_stamp(&db).await?;
    if let Some(v) = cached(&ON_THIS_DAY, today, stamp) {
        return Ok(v);
    }
    let v = db.read_async(move |c| throwback::on_this_day(c, today)).await?;
    *ON_THIS_DAY.lock() = Some((today, stamp, v.clone()));
    Ok(v)
}

/// Home's Time travel shelf (empty with `shown: false` while it's off).
#[tauri::command]
pub async fn home_time_travel(db: State<'_, DbHandle>) -> AppResult<TimeTravel> {
    if !crate::settings::load(&db).home_time_travel {
        return Ok(TimeTravel::default());
    }
    let (today, stamp) = day_and_stamp(&db).await?;
    if let Some(v) = cached(&TIME_TRAVEL, today, stamp) {
        return Ok(v);
    }
    let v = db.read_async(move |c| throwback::time_travel(c, today)).await?;
    *TIME_TRAVEL.lock() = Some((today, stamp, v.clone()));
    Ok(v)
}

fn compare_on(db: &DbHandle) -> AppResult<crate::settings::Settings> {
    let s = crate::settings::load(db);
    if !s.stats_compare {
        return Err(AppError::Invalid("Comparing stats with friends is off in Settings.".into()));
    }
    Ok(s)
}

/// Your summary to share with a friend. `their_unreleased_ok`: the friend
/// agreed to include unreleased songs; they go in only when you did too.
#[tauri::command]
pub async fn stats_compare_summary(db: State<'_, DbHandle>, unreleased_ok: bool, their_unreleased_ok: bool) -> AppResult<CompareSummary> {
    let s = compare_on(&db)?;
    fresh(&db).await?;
    let name = Some(s.listen_name.clone());
    db.read_async(move |c| {
        let today = super::today(c)?;
        compare::summary(c, today, name, unreleased_ok, unreleased_ok && their_unreleased_ok)
    })
    .await
}

/// Summaries friends shared with you (empty until one arrives).
#[tauri::command]
pub async fn stats_friends(db: State<'_, DbHandle>) -> AppResult<Vec<CompareSummary>> {
    compare_on(&db)?;
    db.read_async(compare::friends).await
}

/// Forget a friend's shared summary.
#[tauri::command]
pub async fn stats_friend_remove(db: State<'_, DbHandle>, name: String) -> AppResult<Vec<CompareSummary>> {
    db.write_async(move |c| compare::friend_remove(c, &name)).await
}

/// Compare a friend's summary with yours.
#[tauri::command]
pub async fn stats_compare(db: State<'_, DbHandle>, theirs: CompareSummary, unreleased_ok: bool) -> AppResult<CompareResult> {
    compare_on(&db)?;
    compare::validate(&theirs)?;
    fresh(&db).await?;
    db.read_async(move |c| {
        let today = super::today(c)?;
        let both = unreleased_ok && theirs.unreleased_ok;
        let mine = compare::summary(c, today, None, unreleased_ok, both)?;
        compare::compare(c, &mine, &theirs)
    })
    .await
}

fn wrapped_on(db: &DbHandle) -> AppResult<()> {
    if !crate::settings::load(db).wrapped_top_songs {
        return Err(AppError::Invalid("Wrapped and Top Songs are off in Settings.".into()));
    }
    Ok(())
}

/// Which Top Songs lists and Wrapped years there are, and the year Home shows.
#[tauri::command]
pub async fn top_songs_index(db: State<'_, DbHandle>) -> AppResult<TopSongsIndex> {
    wrapped_on(&db)?;
    fresh(&db).await?;
    db.read_async(|c| {
        let today = super::today(c)?;
        wrapped::index(c, today)
    })
    .await
}

/// A year's Top Songs list, or All-Time without a year.
#[tauri::command]
pub async fn top_songs_list(db: State<'_, DbHandle>, year: Option<i64>) -> AppResult<TopSongsList> {
    wrapped_on(&db)?;
    fresh(&db).await?;
    db.read_async(move |c| {
        let today = super::today(c)?;
        wrapped::list(c, year, today)
    })
    .await
}

/// A year's Wrapped.
#[tauri::command]
pub async fn stats_wrapped(db: State<'_, DbHandle>, year: i64) -> AppResult<Wrapped> {
    wrapped_on(&db)?;
    fresh(&db).await?;
    db.read_async(move |c| wrapped::wrapped(c, year)).await
}
