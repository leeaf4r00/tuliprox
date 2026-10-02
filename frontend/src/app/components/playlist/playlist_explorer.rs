use crate::{
    app::{
        components::{
            menu_item::MenuItem,
            popup_menu::PopupMenu,
            recording::{
                ensure_recording_available, target_name_for_id, PaddingBounds, RecordingForm, RecordingFormPrefill,
            },
            AppIcon, Chip, DropDownIconButton, DropDownOption, DropDownSelection, IconButton, NoContent, Panel,
            Search,
        },
        context::{ConfigContext, PlaylistExplorerContext},
    },
    hooks::{use_clipboard_copy, use_service_context},
    html_if,
    i18n::use_translation,
    model::{BusyStatus, DialogAction, DialogActions, DialogResult, EventMessage},
    services::{CreateRecordingTaskRequest, DialogService, RecordingService, RecordingSourceInput},
};
use shared::{
    model::{
        Permission, PlaylistRequest, PlaylistUrlResolveRequest, SearchRequest, SeriesStreamDetailEpisodeProperties,
        SeriesStreamProperties, UiPlaylistGroup, UiPlaylistItem, VirtualId, XtreamCluster,
    },
    utils::{format_float_localized, Internable},
};
use std::{
    cmp::Ordering,
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    hash::{Hash, Hasher},
    rc::Rc,
    str::FromStr,
};
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use web_sys::{HtmlInputElement, HtmlSelectElement, HtmlVideoElement};
use yew::{platform::spawn_local, prelude::*};

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_namespace = window, js_name = attachTuliproxVideo)]
    fn attach_tuliprox_video(
        video: &HtmlVideoElement,
        url: &str,
        is_hls: bool,
        is_mpeg_ts: bool,
        is_live: bool,
        on_error: &js_sys::Function,
        on_tracks: &js_sys::Function,
    ) -> wasm_bindgen::JsValue;

    #[wasm_bindgen::prelude::wasm_bindgen(js_namespace = window, js_name = detachTuliproxVideo)]
    fn detach_tuliprox_video(handle: &wasm_bindgen::JsValue, video: &HtmlVideoElement);

    #[wasm_bindgen::prelude::wasm_bindgen(js_namespace = window, js_name = setTuliproxVideoQuality)]
    fn set_tuliprox_video_quality(video: &HtmlVideoElement, index: i32);

    #[wasm_bindgen::prelude::wasm_bindgen(js_namespace = window, js_name = setTuliproxVideoAudio)]
    fn set_tuliprox_video_audio(video: &HtmlVideoElement, index: i32);

    #[wasm_bindgen::prelude::wasm_bindgen(js_namespace = window, js_name = setTuliproxVideoSubtitle)]
    fn set_tuliprox_video_subtitle(video: &HtmlVideoElement, index: i32);
}

const TP_EXPLORER_SEARCH_FIELDS_KEY: &str = "tp-explorer-search-fields";
const TP_EXPLORER_SORT_KEY: &str = "tp-explorer-sort";
const TP_EXPLORER_FAVORITE_SERIES_KEY: &str = "tp-explorer-favorite-series";
const TP_EXPLORER_FAVORITE_ITEMS_KEY: &str = "tp-explorer-favorite-items";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlaylistExplorerSort {
    RatingDescending,
    ProviderOrder,
}

impl PlaylistExplorerSort {
    fn from_id(id: &str) -> Self {
        match id {
            "provider" => Self::ProviderOrder,
            _ => Self::RatingDescending,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::RatingDescending => "rating_desc",
            Self::ProviderOrder => "provider",
        }
    }
}

fn load_playlist_explorer_sort() -> PlaylistExplorerSort {
    crate::utils::get_local_storage_item(TP_EXPLORER_SORT_KEY)
        .as_deref()
        .map(PlaylistExplorerSort::from_id)
        .unwrap_or(PlaylistExplorerSort::RatingDescending)
}

fn valid_rating(rating: f64) -> Option<f64> { (rating.is_finite() && rating > 0.001).then_some(rating) }

fn compare_rating_desc(left: f64, right: f64) -> Ordering {
    match (valid_rating(left), valid_rating(right)) {
        (Some(left), Some(right)) => right.partial_cmp(&left).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn sort_by_rating<T>(items: &mut [T], sort: PlaylistExplorerSort, rating: impl Fn(&T) -> f64) {
    if sort == PlaylistExplorerSort::RatingDescending {
        items.sort_by(|left, right| compare_rating_desc(rating(left), rating(right)));
    }
}

fn series_favorite_key(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn playlist_item_favorite_key(item: &UiPlaylistItem) -> String {
    format!("{}:{}:{}:{}", item.xtream_cluster.as_stream_type(), item.virtual_id, item.provider_id, item.input_name)
}

fn browser_player_resume_key(episode_id: u32, title: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    episode_id.hash(&mut hasher);
    title.to_lowercase().hash(&mut hasher);
    format!("tp-browser-player-resume-{:016x}", hasher.finish())
}

fn save_browser_player_position(video: &HtmlVideoElement, key: &str) -> Option<f64> {
    let position = video.current_time();
    let duration = video.duration();
    if !position.is_finite() || !duration.is_finite() || duration <= 0.0 {
        return None;
    }

    if duration - position <= 10.0 {
        crate::utils::remove_local_storage_item(key);
        return None;
    }

    if position >= 5.0 {
        crate::utils::set_local_storage_item(key, &position.to_string());
        return Some(position);
    }

    None
}

fn format_player_time(seconds: f64) -> String {
    let total_seconds = seconds.max(0.0) as u64;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn load_favorite_series() -> HashSet<String> {
    crate::utils::get_local_storage_item(TP_EXPLORER_FAVORITE_SERIES_KEY)
        .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|title| series_favorite_key(&title))
        .filter(|title| !title.is_empty())
        .collect()
}

fn load_favorite_items() -> HashSet<String> {
    crate::utils::get_local_storage_item(TP_EXPLORER_FAVORITE_ITEMS_KEY)
        .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|key| !key.is_empty())
        .collect()
}

#[derive(Clone)]
struct ChannelSelection {
    virtual_id: VirtualId,
    provider_id: String,
    cluster: XtreamCluster,
    downloadable: bool,
    url: String,
    title: String,
    input_name: String,
    series_episodes: Option<Rc<Vec<BrowserPlayerEpisode>>>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Eq, PartialEq, strum_macros::Display, strum_macros::EnumString)]
#[strum(serialize_all = "snake_case")]
enum ExplorerAction {
    PlayInBrowser,
    CopyLinkTuliproxVirtualId,
    CopyLinkTuliproxWebPlayerUrl,
    CopyLinkProviderUrl,
    #[strum(serialize = "download_item")]
    Download,
    #[strum(serialize = "record_item")]
    Record,
}

fn build_download_filename(title: &str, url: &str) -> String {
    let sanitized = title
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' => c,
            _ => '_',
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    let base = if sanitized.is_empty() { "download".to_string() } else { sanitized };
    let ext = url
        .split('?')
        .next()
        .and_then(|base| base.rsplit('/').next())
        .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext))
        .filter(|ext| !ext.is_empty())
        .map_or_else(|| ".mp4".to_string(), |ext| format!(".{ext}"));
    if base.ends_with(&ext) {
        base
    } else {
        format!("{base}{ext}")
    }
}

fn parse_optional_priority_input(priority_value: Option<String>) -> Result<Option<i8>, String> {
    let Some(raw) = priority_value.as_deref() else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed.parse::<i8>().map(Some).map_err(|_| "Priority must be a whole number between -128 and 127".to_string())
}

fn normalize_input_name(input_name: &str) -> Option<String> {
    let trimmed = input_name.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn can_show_download_action(can_write_downloads: bool, selected_channel: Option<&ChannelSelection>) -> bool {
    can_write_downloads && selected_channel.is_some_and(|item| item.cluster != XtreamCluster::Live && item.downloadable)
}

fn can_show_record_action(can_write_recordings: bool, selected_channel: Option<&ChannelSelection>) -> bool {
    can_write_recordings && selected_channel.is_some_and(|item| item.cluster == XtreamCluster::Live)
}

fn can_show_play_action(selected_channel: Option<&ChannelSelection>) -> bool {
    selected_channel.is_some_and(|item| {
        matches!(item.cluster, XtreamCluster::Live | XtreamCluster::Video | XtreamCluster::Series)
    })
}

#[derive(Clone)]
struct SeriesFolder {
    title: String,
    logo: String,
    seasons: BTreeMap<u32, Vec<(u32, Rc<UiPlaylistItem>)>>,
}

enum SeriesExplorerEntry {
    Folder(SeriesFolder),
    Item(Rc<UiPlaylistItem>),
}

fn parse_series_episode_title(title: &str) -> Option<(String, u32, u32)> {
    let pattern = &shared::utils::CONSTANTS.re_episode_code;
    let matched = pattern.captures(title)?.get(0)?;
    let (season, episode) = shared::utils::parse_season_episode(title, pattern)?;
    let series_title = title
        .get(..matched.start())?
        .trim_end_matches(|character: char| {
            character.is_whitespace() || matches!(character, '-' | '_' | '.' | ':' | '|')
        })
        .trim();

    (!series_title.is_empty()).then(|| (series_title.to_string(), season, episode))
}

fn build_series_entries(channels: &[Rc<UiPlaylistItem>]) -> Vec<SeriesExplorerEntry> {
    let mut entries = Vec::new();
    let mut folders = HashMap::<String, usize>::new();

    for channel in channels {
        let Some((title, season, episode)) = parse_series_episode_title(&channel.title) else {
            entries.push(SeriesExplorerEntry::Item(channel.clone()));
            continue;
        };

        let key = title.to_lowercase();
        let index = if let Some(index) = folders.get(&key) {
            *index
        } else {
            let index = entries.len();
            entries.push(SeriesExplorerEntry::Folder(SeriesFolder {
                title,
                logo: channel.logo.to_string(),
                seasons: BTreeMap::new(),
            }));
            folders.insert(key, index);
            index
        };

        if let SeriesExplorerEntry::Folder(folder) = &mut entries[index] {
            if folder.logo.is_empty() && !channel.logo.is_empty() {
                folder.logo = channel.logo.to_string();
            }
            folder.seasons.entry(season).or_default().push((episode, channel.clone()));
        }
    }

    for entry in &mut entries {
        if let SeriesExplorerEntry::Folder(folder) = entry {
            for episodes in folder.seasons.values_mut() {
                episodes.sort_by(|(left_number, left), (right_number, right)| {
                    left_number.cmp(right_number).then_with(|| left.title.cmp(&right.title))
                });
            }
        }
    }

    entries
}

fn series_entry_rating(entry: &SeriesExplorerEntry) -> f64 {
    match entry {
        SeriesExplorerEntry::Item(channel) => channel.rating,
        SeriesExplorerEntry::Folder(folder) => folder
            .seasons
            .values()
            .flat_map(|episodes| episodes.iter())
            .map(|(_, episode)| episode.rating)
            .filter(|rating| valid_rating(*rating).is_some())
            .reduce(f64::max)
            .unwrap_or_default(),
    }
}

fn series_episode_display_title(title: &str, series_title: &str) -> String {
    title
        .strip_prefix(series_title)
        .map(|episode| episode.trim_start_matches(|character: char| {
            character.is_whitespace() || matches!(character, '-' | '_' | '.' | ':' | '|')
        }))
        .filter(|episode| !episode.is_empty())
        .unwrap_or(title)
        .to_string()
}

fn url_indicates_hls(url: &str) -> bool { url.to_ascii_lowercase().contains(".m3u8") }

fn url_indicates_mpeg_ts(url: &str) -> bool {
    let path = url.split(|character| character == '?' || character == '#').next().unwrap_or(url);
    let path = path.to_ascii_lowercase();
    [".ts", ".m2ts", ".mts", ".mpegts"].iter().any(|extension| path.ends_with(extension))
}

#[derive(Clone, Debug, PartialEq)]
struct BrowserPlayerEpisode {
    virtual_id: u32,
    title: String,
    label: String,
    url: String,
    input_name: String,
    season: u32,
    episode: u32,
}

#[derive(Properties, PartialEq)]
struct BrowserPlayerProps {
    title: String,
    src: String,
    virtual_id: u32,
    cluster: XtreamCluster,
    source_url: String,
    input_name: String,
    is_hls: bool,
    is_mpeg_ts: bool,
    is_live: bool,
    current_episode_id: Option<u32>,
    episodes: Vec<BrowserPlayerEpisode>,
    playlist_request: Option<PlaylistRequest>,
    can_download: bool,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
struct BrowserPlayerTrack {
    index: i32,
    label: String,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
struct BrowserPlayerTracks {
    #[serde(default)]
    qualities: Vec<BrowserPlayerTrack>,
    #[serde(default)]
    audio_tracks: Vec<BrowserPlayerTrack>,
    #[serde(default)]
    subtitle_tracks: Vec<BrowserPlayerTrack>,
    #[serde(default = "default_player_track_index")]
    selected_quality: i32,
    #[serde(default = "default_player_track_index")]
    selected_audio: i32,
    #[serde(default = "default_subtitle_track_index")]
    selected_subtitle: i32,
    #[serde(default)]
    source_width: u32,
    #[serde(default)]
    source_height: u32,
}

const fn default_player_track_index() -> i32 { 0 }

const fn default_subtitle_track_index() -> i32 { -1 }

fn localized_player_track_label(
    track: &BrowserPlayerTrack,
    translate: &crate::i18n::YewI18n,
    track_kind: &str,
) -> String {
    if track.index == -1 && track_kind == "quality" {
        translate.t("MESSAGES.PLAYBACK.AUTO")
    } else if track.index == -1 && track_kind == "subtitle" {
        translate.t("MESSAGES.PLAYBACK.SUBTITLES_OFF")
    } else {
        track.label.clone()
    }
}

#[function_component(BrowserPlayer)]
fn browser_player(props: &BrowserPlayerProps) -> Html {
    let translate = use_translation();
    let services = use_service_context();
    let video_ref = use_node_ref();
    let playback_error = use_state(|| false);
    let is_reconnecting = use_state(|| false);
    let pending_resume_position = use_state(|| None::<f64>);
    let recovery_pending = use_mut_ref(|| false);
    let recovery_position = use_mut_ref(|| 0.0_f64);
    let player_tracks = use_state(BrowserPlayerTracks::default);
    let volume = use_state(|| 1.0_f64);
    let muted = use_state(|| false);
    let current_title = use_state(|| props.title.clone());
    let current_src = use_state(|| props.src.clone());
    let current_is_hls = use_state(|| props.is_hls);
    let current_is_mpeg_ts = use_state(|| props.is_mpeg_ts);
    let current_episode_id = use_state(|| props.current_episode_id);
    let current_virtual_id = use_state(|| props.virtual_id);
    let current_cluster = use_state(|| props.cluster);
    let current_source_url = use_state(|| props.source_url.clone());
    let initial_episode_index = props
        .episodes
        .iter()
        .position(|episode| Some(episode.virtual_id) == props.current_episode_id)
        .unwrap_or_default();
    let current_episode_index = use_state(|| initial_episode_index);
    let switching_episode = use_state(|| false);
    let downloading_episode = use_state(|| false);
    let saved_resume_position = use_state(|| 0.0_f64);
    let resume_key = props
        .episodes
        .get(*current_episode_index)
        .map(|episode| browser_player_resume_key(episode.virtual_id, &episode.title))
        .unwrap_or_else(|| browser_player_resume_key(current_episode_id.unwrap_or_default(), &current_title));
    let last_saved_position = use_mut_ref(|| (resume_key.clone(), 0.0_f64));

    {
        let saved_resume_position = saved_resume_position.clone();
        let last_saved_position = last_saved_position.clone();
        use_effect_with(resume_key.clone(), move |key| {
            let saved_position = crate::utils::get_local_storage_item(key)
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|position| position.is_finite() && *position > 10.0)
                .unwrap_or_default();
            saved_resume_position.set(saved_position);
            *last_saved_position.borrow_mut() = (key.clone(), 0.0);
            || {}
        });
    }

    {
        let title = (*current_title).clone();
        use_effect_with(title, move |title| {
            let document = web_sys::window().and_then(|window| window.document());
            let previous_title = document.as_ref().map(web_sys::Document::title);
            if let Some(document) = document.as_ref() {
                document.set_title(title);
            }

            move || {
                if let (Some(document), Some(previous_title)) = (document.as_ref(), previous_title.as_deref()) {
                    document.set_title(previous_title);
                }
            }
        });
    }

    {
        let video_ref = video_ref.clone();
        let playback_error = playback_error.clone();
        let is_reconnecting = is_reconnecting.clone();
        let pending_resume_position = pending_resume_position.clone();
        let recovery_pending = recovery_pending.clone();
        let recovery_position = recovery_position.clone();
        let player_tracks = player_tracks.clone();
        let services = services.clone();
        let playlist_request = props.playlist_request.clone();
        let current_virtual_id = current_virtual_id.clone();
        let current_cluster = current_cluster.clone();
        let current_source_url = current_source_url.clone();
        let current_src = current_src.clone();
        let current_is_hls = current_is_hls.clone();
        let current_is_mpeg_ts = current_is_mpeg_ts.clone();
        let dependencies = (
            (*current_src).clone(),
            *current_is_hls,
            *current_is_mpeg_ts,
            props.is_live,
        );
        use_effect_with(dependencies, move |(src, is_hls, is_mpeg_ts, is_live)| {
            playback_error.set(false);
            player_tracks.set(BrowserPlayerTracks::default());
            let player = video_ref.cast::<HtmlVideoElement>().map(|video| {
                let live_stream = *is_live;
                let playback_error = playback_error.clone();
                let is_reconnecting = is_reconnecting.clone();
                let pending_resume_position = pending_resume_position.clone();
                let recovery_pending = recovery_pending.clone();
                let recovery_position = recovery_position.clone();
                let player_tracks = player_tracks.clone();
                let services = services.clone();
                let playlist_request = playlist_request.clone();
                let current_virtual_id = current_virtual_id.clone();
                let current_cluster = current_cluster.clone();
                let current_source_url = current_source_url.clone();
                let current_src = current_src.clone();
                let current_is_hls = current_is_hls.clone();
                let current_is_mpeg_ts = current_is_mpeg_ts.clone();
                let fallback_src = (*src).clone();
                let callback_is_reconnecting = is_reconnecting.clone();
                let callback_pending_resume_position = pending_resume_position.clone();
                let on_error = Closure::<dyn FnMut(JsValue)>::new(move |resume_position: JsValue| {
                    if live_stream {
                        match resume_position.as_f64() {
                            Some(-1.0) => {
                                playback_error.set(false);
                                callback_is_reconnecting.set(true);
                                callback_pending_resume_position.set(None);
                                return;
                            }
                            Some(-2.0) => {
                                callback_is_reconnecting.set(false);
                                return;
                            }
                            _ => {
                                playback_error.set(true);
                                callback_is_reconnecting.set(false);
                                callback_pending_resume_position.set(None);
                                *recovery_pending.borrow_mut() = false;
                                return;
                            }
                        }
                    }

                    let Some(resume_position) = resume_position.as_f64().filter(|position| {
                        position.is_finite() && *position > 0.0
                    }) else {
                        playback_error.set(true);
                        callback_is_reconnecting.set(false);
                        callback_pending_resume_position.set(None);
                        *recovery_pending.borrow_mut() = false;
                        return;
                    };

                    if *recovery_pending.borrow() {
                        playback_error.set(true);
                        callback_is_reconnecting.set(false);
                        callback_pending_resume_position.set(None);
                        *recovery_pending.borrow_mut() = false;
                        return;
                    }

                    *recovery_pending.borrow_mut() = true;
                    *recovery_position.borrow_mut() = resume_position;
                    callback_pending_resume_position.set(Some(resume_position));
                    callback_is_reconnecting.set(true);

                    let services = services.clone();
                    let playlist_request = playlist_request.clone();
                    let virtual_id = *current_virtual_id;
                    let cluster = *current_cluster;
                    let source_url = (*current_source_url).clone();
                    let fallback_src = fallback_src.clone();
                    let playback_error = playback_error.clone();
                    let is_reconnecting = callback_is_reconnecting.clone();
                    let pending_resume_position = callback_pending_resume_position.clone();
                    let recovery_pending = recovery_pending.clone();
                    let current_src = current_src.clone();
                    let current_is_hls = current_is_hls.clone();
                    let current_is_mpeg_ts = current_is_mpeg_ts.clone();
                    spawn_local(async move {
                        let resolved_url = match playlist_request.as_ref() {
                            Some(PlaylistRequest::Target(target_id)) => {
                                let request = PlaylistUrlResolveRequest::Webplayer {
                                    target_id: *target_id,
                                    virtual_id,
                                    cluster,
                                };
                                services.playlist.resolve_url(request).await.unwrap_or_default()
                            }
                            Some(request) if !source_url.is_empty() => {
                                let resolve_request = PlaylistUrlResolveRequest::Provider {
                                    playlist_request: request.clone(),
                                    url: source_url.clone(),
                                };
                                services.playlist.resolve_url(resolve_request).await.unwrap_or_default()
                            }
                            _ => String::new(),
                        };

                        if resolved_url.is_empty() || resolved_url == fallback_src {
                            playback_error.set(true);
                            is_reconnecting.set(false);
                            pending_resume_position.set(None);
                            *recovery_pending.borrow_mut() = false;
                            return;
                        }

                        let is_hls = url_indicates_hls(&resolved_url);
                        let is_mpeg_ts = !is_hls && url_indicates_mpeg_ts(&resolved_url);
                        current_src.set(resolved_url);
                        current_is_hls.set(is_hls);
                        current_is_mpeg_ts.set(is_mpeg_ts);
                    });
                });
                let on_tracks = Closure::<dyn FnMut(JsValue)>::new(move |payload: JsValue| {
                    let Some(payload) = payload.as_string() else {
                        return;
                    };
                    if let Ok(tracks) = serde_json::from_str::<BrowserPlayerTracks>(&payload) {
                        player_tracks.set(tracks);
                    }
                });
                let restore_handler = if (*pending_resume_position).is_some() {
                    let pending_resume_position = pending_resume_position.clone();
                    let is_reconnecting = is_reconnecting.clone();
                    let restore_video = video.clone();
                    let restore = Closure::<dyn FnMut()>::new(move || {
                        if let Some(position) = *pending_resume_position {
                            let duration = restore_video.duration();
                            let position = if duration.is_finite() && duration > 1.0 {
                                position.min(duration - 1.0)
                            } else {
                                position
                            };
                            restore_video.set_current_time(position);
                            pending_resume_position.set(None);
                            is_reconnecting.set(false);
                            let _ = restore_video.play();
                        }
                    });
                    let _ = video.add_event_listener_with_callback(
                        "loadedmetadata",
                        restore.as_ref().unchecked_ref(),
                    );
                    Some(restore)
                } else {
                    None
                };
                let player_handle = attach_tuliprox_video(
                    &video,
                    src,
                    *is_hls,
                    *is_mpeg_ts,
                    *is_live,
                    on_error.as_ref().unchecked_ref(),
                    on_tracks.as_ref().unchecked_ref(),
                );
                (video, player_handle, on_error, on_tracks, restore_handler)
            });

            move || {
                if let Some((video, player_handle, on_error, on_tracks, restore_handler)) = player {
                    if let Some(restore_handler) = restore_handler {
                        let _ = video.remove_event_listener_with_callback(
                            "loadedmetadata",
                            restore_handler.as_ref().unchecked_ref(),
                        );
                        drop(restore_handler);
                    }
                    detach_tuliprox_video(&player_handle, &video);
                    drop(on_error);
                    drop(on_tracks);
                }
            }
        });
    }

    let on_select_episode = {
        let episodes = props.episodes.clone();
        let playlist_request = props.playlist_request.clone();
        let services = services.clone();
        let translate = translate.clone();
        let current_title = current_title.clone();
        let current_src = current_src.clone();
        let current_is_hls = current_is_hls.clone();
        let current_is_mpeg_ts = current_is_mpeg_ts.clone();
        let current_episode_id = current_episode_id.clone();
        let current_episode_index = current_episode_index.clone();
        let switching_episode = switching_episode.clone();
        let current_virtual_id = current_virtual_id.clone();
        let current_source_url = current_source_url.clone();
        let recovery_pending = recovery_pending.clone();
        let is_reconnecting = is_reconnecting.clone();
        let pending_resume_position = pending_resume_position.clone();
        let playback_error = playback_error.clone();
        let video_ref = video_ref.clone();
        Callback::from(move |index: usize| {
            if *switching_episode || index == *current_episode_index {
                return;
            }
            let Some(episode) = episodes.get(index).cloned() else {
                return;
            };

            switching_episode.set(true);
            playback_error.set(false);
            if let Some(video) = video_ref.cast::<HtmlVideoElement>() {
                let _ = video.pause();
            }

            let services = services.clone();
            let playlist_request = playlist_request.clone();
            let translate = translate.clone();
            let current_title = current_title.clone();
            let current_src = current_src.clone();
            let current_is_hls = current_is_hls.clone();
            let current_is_mpeg_ts = current_is_mpeg_ts.clone();
            let current_episode_id = current_episode_id.clone();
            let current_episode_index = current_episode_index.clone();
            let switching_episode = switching_episode.clone();
            let current_virtual_id = current_virtual_id.clone();
            let current_source_url = current_source_url.clone();
            let recovery_pending = recovery_pending.clone();
            let is_reconnecting = is_reconnecting.clone();
            let pending_resume_position = pending_resume_position.clone();
            spawn_local(async move {
                let resolved_url = match playlist_request.as_ref() {
                    Some(PlaylistRequest::Target(target_id)) => {
                        let request = PlaylistUrlResolveRequest::Webplayer {
                            target_id: *target_id,
                            virtual_id: episode.virtual_id,
                            cluster: XtreamCluster::Series,
                        };
                        services.playlist.resolve_url(request).await.unwrap_or_default()
                    }
                    Some(request) => {
                        let source_url = if !episode.url.is_empty() {
                            episode.url.clone()
                        } else {
                            services
                                .playlist
                                .get_episode(episode.virtual_id, request)
                                .await
                                .map_or_else(String::new, |item| item.url.to_string())
                        };
                        if source_url.is_empty() {
                            String::new()
                        } else {
                            let resolve_request = PlaylistUrlResolveRequest::Provider {
                                playlist_request: request.clone(),
                                url: source_url.clone(),
                            };
                            services
                                .playlist
                                .resolve_url(resolve_request)
                                .await
                                .unwrap_or(source_url)
                        }
                    }
                    None => episode.url.clone(),
                };

                if resolved_url.is_empty() {
                    services.toastr.error(translate.t("MESSAGES.PLAYBACK.NO_URL"));
                    switching_episode.set(false);
                    return;
                }

                let is_hls = url_indicates_hls(&episode.url) || url_indicates_hls(&resolved_url);
                let is_mpeg_ts = !is_hls
                    && (url_indicates_mpeg_ts(&episode.url) || url_indicates_mpeg_ts(&resolved_url));
                current_title.set(episode.title);
                current_src.set(resolved_url);
                current_is_hls.set(is_hls);
                current_is_mpeg_ts.set(is_mpeg_ts);
                current_episode_id.set(Some(episode.virtual_id));
                current_episode_index.set(index);
                current_virtual_id.set(episode.virtual_id);
                current_source_url.set(episode.url);
                pending_resume_position.set(None);
                is_reconnecting.set(false);
                *recovery_pending.borrow_mut() = false;
                switching_episode.set(false);
            });
        })
    };

    let previous_episode_index = (*current_episode_index).checked_sub(1);
    let next_episode_index = (*current_episode_index + 1 < props.episodes.len())
        .then_some(*current_episode_index + 1);
    let on_previous_episode = {
        let on_select_episode = on_select_episode.clone();
        Callback::from(move |_: MouseEvent| {
            if let Some(index) = previous_episode_index {
                on_select_episode.emit(index);
            }
        })
    };
    let on_next_episode = {
        let on_select_episode = on_select_episode.clone();
        Callback::from(move |_: MouseEvent| {
            if let Some(index) = next_episode_index {
                on_select_episode.emit(index);
            }
        })
    };
    let on_video_ended = {
        let on_select_episode = on_select_episode.clone();
        let resume_key = resume_key.clone();
        Callback::from(move |_: Event| {
            crate::utils::remove_local_storage_item(&resume_key);
            if let Some(index) = next_episode_index {
                on_select_episode.emit(index);
            }
        })
    };

    let on_video_time_update = {
        let resume_key = resume_key.clone();
        let last_saved_position = last_saved_position.clone();
        let recovery_pending = recovery_pending.clone();
        let recovery_position = recovery_position.clone();
        Callback::from(move |event: Event| {
            let Some(video) = event.target_dyn_into::<HtmlVideoElement>() else {
                return;
            };
            let position = video.current_time();
            let duration = video.duration();
            let mut last_saved = last_saved_position.borrow_mut();
            if last_saved.0 != resume_key {
                *last_saved = (resume_key.clone(), 0.0);
            }
            if (position - last_saved.1 >= 5.0) || (duration.is_finite() && duration - position <= 10.0) {
                last_saved.1 = save_browser_player_position(&video, &resume_key).unwrap_or_default();
            }
            if *recovery_pending.borrow() && position >= *recovery_position.borrow() + 30.0 {
                *recovery_pending.borrow_mut() = false;
            }
        })
    };
    let on_video_pause = {
        let resume_key = resume_key.clone();
        Callback::from(move |event: Event| {
            if let Some(video) = event.target_dyn_into::<HtmlVideoElement>() {
                save_browser_player_position(&video, &resume_key);
            }
        })
    };
    let on_resume_playback = {
        let video_ref = video_ref.clone();
        let saved_resume_position = saved_resume_position.clone();
        Callback::from(move |_| {
            if let Some(video) = video_ref.cast::<HtmlVideoElement>() {
                video.set_current_time(*saved_resume_position);
                let _ = video.play();
                saved_resume_position.set(0.0);
            }
        })
    };
    let on_download_episode = {
        let episode = props.episodes.get(*current_episode_index).cloned().or_else(|| {
            (!props.is_live && props.cluster == XtreamCluster::Video).then(|| BrowserPlayerEpisode {
                virtual_id: props.virtual_id,
                title: props.title.clone(),
                label: props.title.clone(),
                url: props.source_url.clone(),
                input_name: props.input_name.clone(),
                season: 0,
                episode: 0,
            })
        });
        let playlist_request = props.playlist_request.clone();
        let services = services.clone();
        let translate = translate.clone();
        let downloading_episode = downloading_episode.clone();
        let can_download = props.can_download;
        Callback::from(move |_| {
            if !can_download || *downloading_episode {
                return;
            }
            let Some(episode) = episode.clone() else {
                return;
            };

            downloading_episode.set(true);
            let playlist_request = playlist_request.clone();
            let services = services.clone();
            let translate = translate.clone();
            let downloading_episode = downloading_episode.clone();
            spawn_local(async move {
                let resolved_url = match playlist_request.as_ref() {
                    Some(request) => {
                        let source_url = if !episode.url.is_empty() {
                            episode.url.clone()
                        } else {
                            services
                                .playlist
                                .get_episode(episode.virtual_id, request)
                                .await
                                .map_or_else(String::new, |item| item.url.to_string())
                        };
                        if source_url.is_empty() {
                            String::new()
                        } else {
                            let resolve_request = PlaylistUrlResolveRequest::Provider {
                                playlist_request: request.clone(),
                                url: source_url.clone(),
                            };
                            services.playlist.resolve_url(resolve_request).await.unwrap_or(source_url)
                        }
                    }
                    None => episode.url.clone(),
                };

                if resolved_url.is_empty() || resolved_url.starts_with(shared::utils::PROVIDER_SCHEME_PREFIX) {
                    services.toastr.error(translate.t("MESSAGES.DOWNLOAD.FAIL"));
                    downloading_episode.set(false);
                    return;
                }

                let filename = build_download_filename(&episode.title, &resolved_url);
                let input_name = normalize_input_name(&episode.input_name);
                match services.downloads.queue_download(resolved_url, filename, input_name, None).await {
                    Ok(_) => services.toastr.success(translate.t("MESSAGES.DOWNLOAD.DOWNLOAD_QUEUED")),
                    Err(_) => services.toastr.error(translate.t("MESSAGES.DOWNLOAD.FAIL")),
                }
                downloading_episode.set(false);
            });
        })
    };

    let mut episodes_by_season = BTreeMap::<u32, Vec<(usize, &BrowserPlayerEpisode)>>::new();
    for (index, episode) in props.episodes.iter().enumerate() {
        episodes_by_season.entry(episode.season).or_default().push((index, episode));
    }

    let on_volume_input = {
        let video_ref = video_ref.clone();
        let volume = volume.clone();
        let muted = muted.clone();
        Callback::from(move |event: InputEvent| {
            let Some(input) = event.target_dyn_into::<HtmlInputElement>() else {
                return;
            };
            let next_volume = input.value_as_number().clamp(0.0, 1.0);
            volume.set(next_volume);
            if let Some(video) = video_ref.cast::<HtmlVideoElement>() {
                video.set_volume(next_volume);
                if next_volume > 0.0 {
                    video.set_muted(false);
                    muted.set(false);
                }
            }
        })
    };

    let on_volume_change = {
        let video_ref = video_ref.clone();
        let volume = volume.clone();
        let muted = muted.clone();
        Callback::from(move |_| {
            if let Some(video) = video_ref.cast::<HtmlVideoElement>() {
                volume.set(video.volume());
                muted.set(video.muted());
            }
        })
    };

    let on_toggle_mute = {
        let video_ref = video_ref.clone();
        let muted = muted.clone();
        Callback::from(move |_| {
            if let Some(video) = video_ref.cast::<HtmlVideoElement>() {
                let next_muted = !video.muted();
                video.set_muted(next_muted);
                muted.set(next_muted);
            }
        })
    };

    let on_quality_change = {
        let video_ref = video_ref.clone();
        Callback::from(move |event: Event| {
            let Some(select) = event.target_dyn_into::<HtmlSelectElement>() else {
                return;
            };
            if let Ok(index) = select.value().parse::<i32>() {
                if let Some(video) = video_ref.cast::<HtmlVideoElement>() {
                    set_tuliprox_video_quality(&video, index);
                }
            }
        })
    };

    let on_audio_change = {
        let video_ref = video_ref.clone();
        Callback::from(move |event: Event| {
            let Some(select) = event.target_dyn_into::<HtmlSelectElement>() else {
                return;
            };
            if let Ok(index) = select.value().parse::<i32>() {
                if let Some(video) = video_ref.cast::<HtmlVideoElement>() {
                    set_tuliprox_video_audio(&video, index);
                }
            }
        })
    };

    let on_subtitle_change = {
        let video_ref = video_ref.clone();
        Callback::from(move |event: Event| {
            let Some(select) = event.target_dyn_into::<HtmlSelectElement>() else {
                return;
            };
            if let Ok(index) = select.value().parse::<i32>() {
                if let Some(video) = video_ref.cast::<HtmlVideoElement>() {
                    set_tuliprox_video_subtitle(&video, index);
                }
            }
        })
    };

    html! {
        <div class="tp__browser-player">
            <h2 class="tp__browser-player__title">{(*current_title).clone()}</h2>
            <div class="tp__browser-player__layout">
              <div class="tp__browser-player__main">
                <video
                    class="tp__browser-player__video"
                    ref={video_ref}
                    controls=true
                    autoplay=true
                    playsinline=true
                    preload="metadata"
                    aria-label={(*current_title).clone()}
                    onvolumechange={on_volume_change}
                    ontimeupdate={on_video_time_update}
                    onpause={on_video_pause}
                    onended={on_video_ended}
                />
                <div class="tp__browser-player__controls" aria-label={translate.t("MESSAGES.PLAYBACK.CONTROLS")}>
                <div class="tp__browser-player__volume">
                    <label for="tp-player-volume">{translate.t("MESSAGES.PLAYBACK.VOLUME")}</label>
                    <input
                        id="tp-player-volume"
                        type="range"
                        min="0"
                        max="1"
                        step="0.01"
                        value={volume.to_string()}
                        aria-label={translate.t("MESSAGES.PLAYBACK.VOLUME")}
                        oninput={on_volume_input}
                    />
                    <span>{format!("{}%", ((*volume * 100.0).round() as u32))}</span>
                    <button type="button" onclick={on_toggle_mute} aria-label={if *muted {
                        translate.t("MESSAGES.PLAYBACK.UNMUTE")
                    } else {
                        translate.t("MESSAGES.PLAYBACK.MUTE")
                    }}>{if *muted {
                        translate.t("MESSAGES.PLAYBACK.UNMUTE")
                    } else {
                        translate.t("MESSAGES.PLAYBACK.MUTE")
                    }}</button>
                </div>
                {if props.can_download && (!props.episodes.is_empty() || props.cluster == XtreamCluster::Video) {
                    html! {
                        <button
                            type="button"
                            class="tp__browser-player__download-button"
                            disabled={*switching_episode || *downloading_episode}
                            onclick={on_download_episode}
                            aria-label={translate.t(if props.cluster == XtreamCluster::Video { "LABEL.DOWNLOAD" } else { "MESSAGES.PLAYBACK.DOWNLOAD_EPISODE" })}
                        >
                            <AppIcon name="Download" />
                            {if *downloading_episode {
                                translate.t("MESSAGES.PLAYBACK.DOWNLOADING_EPISODE")
                            } else {
                                translate.t(if props.cluster == XtreamCluster::Video { "LABEL.DOWNLOAD" } else { "MESSAGES.PLAYBACK.DOWNLOAD_EPISODE" })
                            }}
                        </button>
                    }
                } else {
                    Html::default()
                }}
                {if !player_tracks.qualities.is_empty() {
                    html! {
                        <label class="tp__browser-player__select">
                            <span>{translate.t("MESSAGES.PLAYBACK.QUALITY")}</span>
                            <select
                                value={player_tracks.selected_quality.to_string()}
                                disabled={player_tracks.qualities.len() <= 1}
                                onchange={on_quality_change.clone()}
                                aria-label={translate.t("MESSAGES.PLAYBACK.QUALITY")}
                            >
                                {for player_tracks.qualities.iter().map(|track| html! {
                                    <option value={track.index.to_string()} selected={track.index == player_tracks.selected_quality}>
                                        {localized_player_track_label(track, &translate, "quality")}
                                    </option>
                                })}
                            </select>
                        </label>
                    }
                } else {
                    Html::default()
                }}
                {if !player_tracks.audio_tracks.is_empty() {
                    html! {
                        <label class="tp__browser-player__select">
                            <span>{translate.t("MESSAGES.PLAYBACK.AUDIO")}</span>
                            <select
                                value={player_tracks.selected_audio.to_string()}
                                disabled={player_tracks.audio_tracks.len() <= 1}
                                onchange={on_audio_change.clone()}
                                aria-label={translate.t("MESSAGES.PLAYBACK.AUDIO")}
                            >
                                {for player_tracks.audio_tracks.iter().map(|track| html! {
                                    <option value={track.index.to_string()} selected={track.index == player_tracks.selected_audio}>
                                        {track.label.clone()}
                                    </option>
                                })}
                            </select>
                        </label>
                    }
                } else {
                    Html::default()
                }}
                {if !player_tracks.subtitle_tracks.is_empty() {
                    html! {
                        <label class="tp__browser-player__select">
                            <span>{translate.t("MESSAGES.PLAYBACK.SUBTITLES")}</span>
                            <select
                                value={player_tracks.selected_subtitle.to_string()}
                                disabled={player_tracks.subtitle_tracks.len() <= 1}
                                onchange={on_subtitle_change.clone()}
                                aria-label={translate.t("MESSAGES.PLAYBACK.SUBTITLES")}
                            >
                                {for player_tracks.subtitle_tracks.iter().map(|track| html! {
                                    <option value={track.index.to_string()} selected={track.index == player_tracks.selected_subtitle}>
                                        {localized_player_track_label(track, &translate, "subtitle")}
                                    </option>
                                })}
                            </select>
                        </label>
                    }
                } else {
                    Html::default()
                }}
                </div>
                {if *saved_resume_position > 10.0 && !props.is_live {
                    html! {
                        <button
                            type="button"
                            class="tp__browser-player__resume-button"
                            onclick={on_resume_playback}
                        >
                            <AppIcon name="PlayArrow" />
                            {translate.t("MESSAGES.PLAYBACK.RESUME_FROM")}
                            {" · "}
                            {format_player_time(*saved_resume_position)}
                        </button>
                    }
                } else {
                    Html::default()
                }}
                {if *is_reconnecting {
                    html! { <p class="tp__browser-player__hint" role="status">{translate.t("MESSAGES.PLAYBACK.RECONNECTING")}</p> }
                } else {
                    Html::default()
                }}
                <p class="tp__browser-player__hint">{translate.t("MESSAGES.PLAYBACK.BROWSER_HINT")}</p>
                {if *playback_error {
                    html! { <p class="tp__browser-player__error" role="alert">{translate.t("MESSAGES.PLAYBACK.ERROR")}</p> }
                } else {
                    Html::default()
                }}
              </div>
              {if props.episodes.is_empty() {
                  Html::default()
              } else {
                  html! {
                    <aside class="tp__browser-player__episodes" aria-label={translate.t("MESSAGES.PLAYBACK.EPISODES")}>
                        <div class="tp__browser-player__episodes-header">
                            <h3>{translate.t("MESSAGES.PLAYBACK.EPISODES")}</h3>
                            <span>{props.episodes.len()}</span>
                        </div>
                        {if props.episodes.len() > 1 {
                            html! {
                                <nav class="tp__browser-player__episode-navigation" aria-label={translate.t("MESSAGES.PLAYBACK.EPISODE_NAVIGATION")}>
                                    <button
                                        type="button"
                                        disabled={previous_episode_index.is_none() || *switching_episode}
                                        onclick={on_previous_episode.clone()}
                                    >{translate.t("MESSAGES.PLAYBACK.PREVIOUS_EPISODE")}</button>
                                    <span>{format!("{} / {}", *current_episode_index + 1, props.episodes.len())}</span>
                                    <button
                                        type="button"
                                        disabled={next_episode_index.is_none() || *switching_episode}
                                        onclick={on_next_episode.clone()}
                                    >{translate.t("MESSAGES.PLAYBACK.NEXT_EPISODE")}</button>
                                </nav>
                            }
                        } else {
                            Html::default()
                        }}
                        {if *switching_episode {
                            html! { <p class="tp__browser-player__episode-status" role="status">{translate.t("MESSAGES.PLAYBACK.EPISODE_LOADING")}</p> }
                        } else {
                            Html::default()
                        }}
                        <div class="tp__browser-player__episode-list">
                            {for episodes_by_season.iter().map(|(season, season_episodes)| {
                                html! {
                                    <section class="tp__browser-player__episode-season" key={format!("season-{season}")}>
                                        <h4>{format!("{} - {}", translate.t("LABEL.SEASON"), season)}</h4>
                                        <div class="tp__browser-player__episode-items">
                                            {for season_episodes.iter().map(|(index, episode)| {
                                                let index = *index;
                                                let select_episode = on_select_episode.clone();
                                                let on_click = Callback::from(move |_| select_episode.emit(index));
                                                let is_current = *current_episode_id == Some(episode.virtual_id);
                                                html! {
                                                    <button
                                                        type="button"
                                                        key={episode.virtual_id.to_string()}
                                                        class={if is_current { "tp__browser-player__episode is-current" } else { "tp__browser-player__episode" }}
                                                        aria-current={is_current.to_string()}
                                                        aria-label={episode.title.clone()}
                                                        title={episode.title.clone()}
                                                        disabled={*switching_episode}
                                                        onclick={on_click}
                                                    >
                                                        <span class="tp__browser-player__episode-number">{format!("{:02}·{:02}", episode.season, episode.episode)}</span>
                                                        <span class="tp__browser-player__episode-label">{episode.label.clone()}</span>
                                                    </button>
                                                }
                                            })}
                                        </div>
                                    </section>
                                }
                            })}
                        </div>
                    </aside>
                  }
              }}
            </div>
        </div>
    }
}

enum ExplorerLevel {
    Categories,
    Group(Rc<UiPlaylistGroup>),
    SeriesFolder(Rc<UiPlaylistGroup>, Rc<SeriesFolder>),
    SeriesInfo(Rc<UiPlaylistGroup>, Rc<UiPlaylistItem>, Option<Box<SeriesStreamProperties>>),
    Favorites,
}

#[component]
pub fn PlaylistExplorer() -> Html {
    let explorer_ctx = use_context::<PlaylistExplorerContext>();
    let cfg_ctx = use_context::<ConfigContext>();
    let dialog_ctx = use_context::<DialogService>();
    let translate = use_translation();
    // Render a fallback instead of panicking when a provider is missing
    let (Some(context), Some(config_ctx), Some(dialog)) = (explorer_ctx, cfg_ctx, dialog_ctx) else {
        log::error!("PlaylistExplorer rendered without required context providers");
        return html! { <NoContent text={translate.t("LABEL.NO_CONTENT")} /> };
    };
    let service_ctx = use_service_context();
    let can_write_downloads = service_ctx.auth.has_permission(Permission::DownloadWrite);
    let can_write_recordings = service_ctx.auth.has_permission(Permission::RecordingWrite);
    let is_admin_role = service_ctx.auth.is_admin();
    let default_download_priority = config_ctx
        .config
        .as_ref()
        .and_then(|cfg| cfg.config.video.as_ref())
        .and_then(|video| video.download.as_ref())
        .map(|download| download.download_priority);
    let recording_padding = {
        let rec = config_ctx
            .config
            .as_ref()
            .and_then(|cfg| cfg.config.video.as_ref())
            .and_then(|video| video.download.as_ref())
            .and_then(|video| video.recording.as_ref());
        PaddingBounds {
            default_pre_roll_secs: rec.and_then(|c| c.default_pre_roll_secs).unwrap_or(0),
            max_pre_roll_secs: rec.map_or(900, |c| c.max_pre_roll_secs),
            default_post_roll_secs: rec.and_then(|c| c.default_post_roll_secs).unwrap_or(0),
            max_post_roll_secs: rec.map_or(1800, |c| c.max_post_roll_secs),
        }
    };
    let recording_padding = Rc::new(recording_padding);
    let current_item = use_state(|| ExplorerLevel::Categories);
    let playlist = use_state(|| (*context.playlist).clone());
    let playlist_sort = use_state(load_playlist_explorer_sort);
    let favorite_series = use_state(load_favorite_series);
    let favorite_items = use_state(load_favorite_items);
    let handle_toggle_series_favorite = {
        let favorite_series = favorite_series.clone();
        Callback::from(move |title: String| {
            let key = series_favorite_key(&title);
            if key.is_empty() {
                return;
            }
            let mut next = (*favorite_series).clone();
            if !next.remove(&key) {
                next.insert(key);
            }
            let mut persisted = next.iter().cloned().collect::<Vec<_>>();
            persisted.sort();
            if let Ok(value) = serde_json::to_string(&persisted) {
                crate::utils::set_local_storage_item(TP_EXPLORER_FAVORITE_SERIES_KEY, &value);
            }
            favorite_series.set(next);
        })
    };
    let handle_toggle_item_favorite = {
        let favorite_items = favorite_items.clone();
        Callback::from(move |key: String| {
            if key.is_empty() {
                return;
            }
            let mut next = (*favorite_items).clone();
            if !next.remove(&key) {
                next.insert(key);
            }
            let mut persisted = next.iter().cloned().collect::<Vec<_>>();
            persisted.sort();
            if let Ok(value) = serde_json::to_string(&persisted) {
                crate::utils::set_local_storage_item(TP_EXPLORER_FAVORITE_ITEMS_KEY, &value);
            }
            favorite_items.set(next);
        })
    };
    let handle_favorites_toggle = {
        let current_item = current_item.clone();
        Callback::from(move |(_name, _event): (String, MouseEvent)| {
            if matches!(&*current_item, ExplorerLevel::Favorites) {
                current_item.set(ExplorerLevel::Categories);
            } else {
                current_item.set(ExplorerLevel::Favorites);
            }
        })
    };
    let search_fields = use_memo((), |()| {
        let persisted: Vec<String> = crate::utils::get_local_storage_item(TP_EXPLORER_SEARCH_FIELDS_KEY)
            .map(|value| value.split(',').filter(|id| !id.is_empty()).map(str::to_string).collect())
            .unwrap_or_default();
        let is_selected = |id: &str| persisted.iter().any(|p| p == id);
        vec![
            DropDownOption::new(
                shared::model::SEARCH_FIELD_GROUP,
                html! { translate.t("LABEL.GROUP") },
                is_selected(shared::model::SEARCH_FIELD_GROUP),
            ),
            DropDownOption::new(
                shared::model::SEARCH_FIELD_TITLE,
                html! { translate.t("LABEL.TITLE") },
                is_selected(shared::model::SEARCH_FIELD_TITLE),
            ),
            DropDownOption::new(
                shared::model::SEARCH_FIELD_NAME,
                html! { translate.t("LABEL.NAME") },
                is_selected(shared::model::SEARCH_FIELD_NAME),
            ),
            DropDownOption::new(
                shared::model::SEARCH_FIELD_RATING,
                html! { translate.t("LABEL.RATING") },
                is_selected(shared::model::SEARCH_FIELD_RATING),
            ),
            DropDownOption::new(
                shared::model::SEARCH_FIELD_URL,
                html! { translate.t("LABEL.URL") },
                is_selected(shared::model::SEARCH_FIELD_URL),
            ),
        ]
    });
    let sort_options = Rc::new(vec![
        DropDownOption::new(
            "rating_desc",
            html! { translate.t("LABEL.SORT_RATING_DESC") },
            *playlist_sort == PlaylistExplorerSort::RatingDescending,
        ),
        DropDownOption::new(
            "provider",
            html! { translate.t("LABEL.SORT_SOURCE_ORDER") },
            *playlist_sort == PlaylistExplorerSort::ProviderOrder,
        ),
    ]);
    let handle_sort_change = {
        let playlist_sort = playlist_sort.clone();
        Callback::from(move |(_name, selection): (String, DropDownSelection)| {
            let DropDownSelection::Single(id) = selection else {
                return;
            };
            let sort = PlaylistExplorerSort::from_id(&id);
            crate::utils::set_local_storage_item(TP_EXPLORER_SORT_KEY, sort.id());
            playlist_sort.set(sort);
        })
    };
    let handle_search_fields_change = Callback::from(move |fields: Option<Rc<Vec<String>>>| {
        let value = fields.as_ref().map(|f| f.join(",")).unwrap_or_default();
        crate::utils::set_local_storage_item(TP_EXPLORER_SEARCH_FIELDS_KEY, &value);
    });
    let selected_channel = use_state(|| None::<ChannelSelection>);
    let popup_anchor_ref = use_state(|| None::<web_sys::Element>);
    let popup_is_open = use_state(|| false);
    let copy_to_clipboard = use_clipboard_copy();
    let cluster_visible = use_state(|| XtreamCluster::Live);

    let handle_cluster_change = {
        let cluster_vis = cluster_visible.clone();
        Callback::from(move |(name, _event): (String, MouseEvent)| {
            if let Ok(xc) = XtreamCluster::from_str(name.as_str()) {
                cluster_vis.set(xc);
            }
        })
    };

    let handle_popup_close = {
        let set_is_open = popup_is_open.clone();
        Callback::from(move |()| {
            set_is_open.set(false);
        })
    };

    let handle_popup_onclick = {
        let set_selected_channel = selected_channel.clone();
        let set_anchor_ref = popup_anchor_ref.clone();
        let set_is_open = popup_is_open.clone();
        Callback::from(move |(dto, event): (Rc<UiPlaylistItem>, MouseEvent)| {
            event.prevent_default();
            event.stop_propagation();
            if let Some(target) = event.target_dyn_into::<web_sys::Element>() {
                set_selected_channel.set(Some(ChannelSelection {
                    virtual_id: VirtualId::new(dto.virtual_id),
                    provider_id: dto.provider_id.to_string(),
                    cluster: dto.xtream_cluster,
                    downloadable: dto.xtream_cluster == XtreamCluster::Video,
                    url: dto.url.to_string(),
                    title: dto.title.to_string(),
                    input_name: dto.input_name.to_string(),
                    series_episodes: None,
                }));
                set_anchor_ref.set(Some(target));
                set_is_open.set(true);
            }
        })
    };

    let handle_episode_popup_onclick = {
        let set_selected_channel = selected_channel.clone();
        let set_anchor_ref = popup_anchor_ref.clone();
        let set_is_open = popup_is_open.clone();
        Callback::from(move |(dto, event): (ChannelSelection, MouseEvent)| {
            event.prevent_default();
            event.stop_propagation();
            if let Some(target) = event.target_dyn_into::<web_sys::Element>() {
                set_selected_channel.set(Some(dto));
                set_anchor_ref.set(Some(target));
                set_is_open.set(true);
            }
        })
    };

    let load_series_info = {
        let set_current_item = current_item.clone();
        let services = service_ctx.clone();
        let ctx = context.clone();

        move |group: Rc<UiPlaylistGroup>, dto: Rc<UiPlaylistItem>| {
            // UiPlaylistItem has no additional_properties - always load from server
            let set_current_item = set_current_item.clone();
            let services = services.clone();
            let ctx = ctx.clone();
            services.event.broadcast(EventMessage::Busy(BusyStatus::Show));
            spawn_local(async move {
                let mut handled = false;
                if let Some(playlist_request) = ctx.playlist_request.as_ref() {
                    if let Some(props) = services.playlist.get_series_info(&dto, playlist_request).await {
                        handled = true;
                        set_current_item.set(ExplorerLevel::SeriesInfo(
                            group.clone(),
                            dto.clone(),
                            Some(Box::new(props)),
                        ));
                    }
                }
                if !handled {
                    set_current_item.set(ExplorerLevel::SeriesInfo(group, dto, None));
                }
                services.event.broadcast(EventMessage::Busy(BusyStatus::Hide));
            });
        }
    };

    {
        let set_playlist = playlist.clone();
        let set_current_item = current_item.clone();
        let set_selected_channel = selected_channel.clone();
        let set_popup_is_open = popup_is_open.clone();
        let set_anchor_ref = popup_anchor_ref.clone();
        use_effect_with((*context.playlist).clone(), move |new_playlist| {
            set_current_item.set(ExplorerLevel::Categories);
            set_playlist.set(new_playlist.clone());
            // Reset popup state and selection when the underlying data changes
            set_selected_channel.set(None);
            set_popup_is_open.set(false);
            set_anchor_ref.set(None);
            || {}
        });
    }

    let handle_menu_click = {
        let services = service_ctx.clone();
        let dialog = dialog.clone();
        let popup_is_open_state = popup_is_open.clone();
        let selected_channel = selected_channel.clone();
        let playlist_ctx = context.clone();
        let translate_clone = translate.clone();
        let can_queue_downloads = can_write_downloads;
        let copy_to_clipboard = copy_to_clipboard.clone();
        let config = config_ctx.config.clone();
        Callback::from(move |(name, _): (String, _)| {
            if let Ok(action) = ExplorerAction::from_str(&name) {
                match action {
                    ExplorerAction::PlayInBrowser => {
                        if let Some(dto) = &*selected_channel {
                            let dialog = dialog.clone();
                            let services = services.clone();
                            let translate_clone = translate_clone.clone();
                            let playlist_request = (*playlist_ctx.playlist_request).clone();
                            let selected = dto.clone();

                            spawn_local(async move {
                                let mut player_title = selected.title.clone();
                                let mut player_virtual_id = selected.virtual_id.get();
                                let mut player_source_url = selected.url.clone();
                                let mut episodes = selected
                                    .series_episodes
                                    .as_ref()
                                    .map_or_else(Vec::new, |episodes| episodes.as_ref().clone());

                                // A provider series card represents the whole series. Load its
                                // episodes before opening the player so the player doesn't treat
                                // the series card as a single episode.
                                if selected.cluster == XtreamCluster::Series && episodes.is_empty() {
                                    if let Some(request) = playlist_request.as_ref() {
                                        if let Some(properties) = services
                                            .playlist
                                            .get_series_info_by_id(
                                                selected.virtual_id.get(),
                                                &selected.provider_id,
                                                request,
                                            )
                                            .await
                                        {
                                            if let Some(series_episodes) =
                                                properties.details.and_then(|details| details.episodes)
                                            {
                                                episodes = series_episodes
                                                    .into_iter()
                                                    .map(|episode| BrowserPlayerEpisode {
                                                        virtual_id: episode.id,
                                                        label: series_episode_display_title(
                                                            &episode.title,
                                                            &selected.title,
                                                        ),
                                                        title: episode.title.to_string(),
                                                        url: episode.direct_source.to_string(),
                                                        input_name: selected.input_name.clone(),
                                                        season: episode.season,
                                                        episode: episode.episode_num,
                                                    })
                                                    .collect();
                                                episodes.sort_by_key(|episode| {
                                                    (episode.season, episode.episode, episode.virtual_id)
                                                });
                                                if let Some(first_episode) = episodes.first() {
                                                    player_title = first_episode.title.clone();
                                                    player_virtual_id = first_episode.virtual_id;
                                                    player_source_url = first_episode.url.clone();
                                                }
                                            }
                                        }
                                    }
                                }

                                if selected.cluster == XtreamCluster::Series && player_source_url.is_empty() {
                                    if let Some(request) = playlist_request.as_ref() {
                                        player_source_url = services
                                            .playlist
                                            .get_episode(player_virtual_id, request)
                                            .await
                                            .map_or_else(String::new, |episode| episode.url.to_string());
                                    }
                                }

                                let resolved_url = match playlist_request.as_ref() {
                                    Some(PlaylistRequest::Target(target_id)) => {
                                        let request = PlaylistUrlResolveRequest::Webplayer {
                                            target_id: *target_id,
                                            virtual_id: player_virtual_id,
                                            cluster: selected.cluster,
                                        };
                                        services.playlist.resolve_url(request).await.unwrap_or_default()
                                    }
                                    Some(request) => {
                                        let source_url = if !player_source_url.is_empty() {
                                            player_source_url.clone()
                                        } else if selected.cluster == XtreamCluster::Series {
                                            services
                                                .playlist
                                                .get_episode(player_virtual_id, request)
                                                .await
                                                .map_or_else(String::new, |episode| episode.url.to_string())
                                        } else {
                                            String::new()
                                        };

                                        if source_url.is_empty() {
                                            String::new()
                                        } else {
                                            let resolve_request = PlaylistUrlResolveRequest::Provider {
                                                playlist_request: request.clone(),
                                                url: source_url.clone(),
                                            };
                                            services
                                                .playlist
                                                .resolve_url(resolve_request)
                                                .await
                                                .unwrap_or(source_url)
                                        }
                                    }
                                    None => player_source_url.clone(),
                                };

                                if resolved_url.is_empty() {
                                    services.toastr.error(translate_clone.t("MESSAGES.PLAYBACK.NO_URL"));
                                    return;
                                }

                                let is_hls = url_indicates_hls(&player_source_url) || url_indicates_hls(&resolved_url);
                                let is_mpeg_ts = !is_hls
                                    && (url_indicates_mpeg_ts(&player_source_url)
                                        || url_indicates_mpeg_ts(&resolved_url));
                                let current_episode_id = (selected.cluster == XtreamCluster::Series)
                                    .then_some(player_virtual_id);
                                let current_source_url = episodes
                    .iter()
                    .find(|episode| Some(episode.virtual_id) == current_episode_id)
                                    .map(|episode| episode.url.clone())
                                    .filter(|url| !url.is_empty())
                                    .unwrap_or_else(|| player_source_url.clone());
                                let content = html! {
                                    <BrowserPlayer
                                        title={player_title}
                                        src={resolved_url}
                                        virtual_id={player_virtual_id}
                                        cluster={selected.cluster}
                                        source_url={current_source_url}
                                        input_name={selected.input_name.clone()}
                                        is_hls={is_hls}
                                        is_mpeg_ts={is_mpeg_ts}
                                        is_live={selected.cluster == XtreamCluster::Live}
                                        current_episode_id={current_episode_id}
                                        episodes={episodes}
                                        playlist_request={playlist_request}
                                        can_download={can_write_downloads}
                                    />
                                };
                                let _ = dialog.content(content, None, true).await;
                            });
                        }
                    }
                    ExplorerAction::CopyLinkTuliproxVirtualId => {
                        if let Some(dto) = &*selected_channel {
                            copy_to_clipboard.emit(dto.virtual_id.to_string());
                        }
                    }
                    ExplorerAction::CopyLinkTuliproxWebPlayerUrl => {
                        if let Some(playlist_request) = playlist_ctx.playlist_request.as_ref() {
                            match playlist_request {
                                PlaylistRequest::Target(target_id) => {
                                    if let Some(dto) = &*selected_channel {
                                        let copy_to_clipboard = copy_to_clipboard.clone();
                                        let services = services.clone();
                                        let virtual_id = dto.virtual_id;
                                        let cluster = dto.cluster;
                                        let translate_clone = translate_clone.clone();
                                        let target_id = *target_id;
                                        let services_clone = services.clone();
                                        spawn_local(async move {
                                            let request = PlaylistUrlResolveRequest::Webplayer {
                                                target_id,
                                                virtual_id: virtual_id.get(),
                                                cluster,
                                            };
                                            if let Some(url) = services.playlist.resolve_url(request).await {
                                                copy_to_clipboard.emit(url);
                                                services_clone.toastr.success(
                                                    translate_clone
                                                        .t("MESSAGES.PLAYLIST.WEBPLAYER_URL_COPY_TO_CLIPBOARD"),
                                                );
                                            } else {
                                                services_clone.toastr.error(
                                                    translate_clone.t("MESSAGES.FAILED_TO_RETRIEVE_WEBPLAYER_URL"),
                                                );
                                            }
                                        });
                                    }
                                }
                                PlaylistRequest::Input(_) => {}
                                PlaylistRequest::CustomXtream(_) => {}
                                PlaylistRequest::CustomM3u(_) => {}
                            }
                        }
                    }
                    ExplorerAction::CopyLinkProviderUrl => {
                        if let Some(dto) = &*selected_channel {
                            let url = dto.url.clone();
                            if url.is_empty() {
                                // Try to fetch episode
                                if let Some(playlist_request) = playlist_ctx.playlist_request.as_ref() {
                                    let copy_to_clipboard = copy_to_clipboard.clone();
                                    let services = services.clone();
                                    let virtual_id = dto.virtual_id;
                                    let playlist_request = playlist_request.clone();
                                    spawn_local(async move {
                                        if let Some(pli) =
                                            services.playlist.get_episode(virtual_id.get(), &playlist_request).await
                                        {
                                            let url = pli.url.to_string();
                                            let request = PlaylistUrlResolveRequest::Provider {
                                                playlist_request,
                                                url: url.clone(),
                                            };
                                            let resolved = services.playlist.resolve_url(request).await.unwrap_or(url);
                                            copy_to_clipboard.emit(resolved);
                                        }
                                    });
                                }
                            } else if let Some(playlist_request) = playlist_ctx.playlist_request.as_ref() {
                                let copy_to_clipboard = copy_to_clipboard.clone();
                                let services = services.clone();
                                let playlist_request = playlist_request.clone();
                                spawn_local(async move {
                                    let request =
                                        PlaylistUrlResolveRequest::Provider { playlist_request, url: url.clone() };
                                    let resolved =
                                        services.playlist.resolve_url(request).await.unwrap_or_else(|| url.clone());
                                    copy_to_clipboard.emit(resolved);
                                });
                            } else {
                                copy_to_clipboard.emit(url);
                            }
                        }
                    }
                    ExplorerAction::Download => {
                        if !can_queue_downloads {
                            popup_is_open_state.set(false);
                            return;
                        }
                        if let Some(dto) = &*selected_channel {
                            let dialog = dialog.clone();
                            let services = services.clone();
                            let translate_clone = translate_clone.clone();
                            let playlist_request = (*playlist_ctx.playlist_request).clone();
                            let default_download_priority = default_download_priority;
                            let selected = dto.clone();
                            spawn_local(async move {
                                let resolved_url = if !selected.url.is_empty() {
                                    if let Some(playlist_request) = playlist_request.clone() {
                                        let request = PlaylistUrlResolveRequest::Provider {
                                            playlist_request,
                                            url: selected.url.clone(),
                                        };
                                        services.playlist.resolve_url(request).await.unwrap_or(selected.url.clone())
                                    } else {
                                        selected.url.clone()
                                    }
                                } else if selected.cluster == XtreamCluster::Series {
                                    if let Some(playlist_request) = playlist_request.as_ref() {
                                        if let Some(pli) = services
                                            .playlist
                                            .get_episode(selected.virtual_id.get(), playlist_request)
                                            .await
                                        {
                                            let episode_url = pli.url.to_string();
                                            let request = PlaylistUrlResolveRequest::Provider {
                                                playlist_request: playlist_request.clone(),
                                                url: episode_url.clone(),
                                            };
                                            services.playlist.resolve_url(request).await.unwrap_or(episode_url)
                                        } else {
                                            String::new()
                                        }
                                    } else {
                                        String::new()
                                    }
                                } else {
                                    String::new()
                                };

                                if resolved_url.is_empty() {
                                    services.toastr.error(translate_clone.t("MESSAGES.DOWNLOAD.FAIL"));
                                    return;
                                }

                                let default_filename = build_download_filename(&selected.title, &resolved_url);
                                let filename_value = Rc::new(RefCell::new(default_filename.clone()));
                                let default_download_priority_value =
                                    default_download_priority.map_or_else(String::new, |priority| priority.to_string());
                                let priority_value = Rc::new(RefCell::new(default_download_priority_value.clone()));
                                let actions = DialogActions {
                                    left: Some(vec![DialogAction::new(
                                        "cancel",
                                        "LABEL.CANCEL",
                                        DialogResult::Cancel,
                                        Some("Close".to_owned()),
                                        None,
                                    )]),
                                    right: vec![DialogAction::new_focused(
                                        "download",
                                        "LABEL.DOWNLOAD",
                                        DialogResult::Ok,
                                        Some("Download".to_owned()),
                                        Some("primary".to_string()),
                                    )],
                                };
                                let filename_value_input = Rc::clone(&filename_value);
                                let priority_value_input = Rc::clone(&priority_value);
                                let result = dialog
                                    .content(
                                        html! {
                                            <div class="tp__record-dialog">
                                                <div class="tp__input">
                                                    <label class="tp__label">{translate_clone.t("LABEL.FILENAME")}</label>
                                                    <div class="tp__input-wrapper">
                                                        <input
                                                            type="text"
                                                            value={default_filename.clone()}
                                                            oninput={Callback::from(move |event: InputEvent| {
                                                                let input: HtmlInputElement = event.target_unchecked_into();
                                                                *filename_value_input.borrow_mut() = input.value();
                                                            })}
                                                        />
                                                    </div>
                                                </div>
                                                <div class="tp__input">
                                                    <label class="tp__label">{translate_clone.t("LABEL.PRIORITY")}</label>
                                                    <div class="tp__input-wrapper">
                                                        <input
                                                            type="number"
                                                            min="-127"
                                                            max="127"
                                                            step="1"
                                                            value={default_download_priority_value.clone()}
                                                            oninput={Callback::from(move |event: InputEvent| {
                                                                let input: HtmlInputElement = event.target_unchecked_into();
                                                                *priority_value_input.borrow_mut() = input.value();
                                                            })}
                                                        />
                                                    </div>
                                                </div>
                                                <div class="tp__field-explanation">
                                                    {selected.title.clone()}
                                                </div>
                                            </div>
                                        },
                                        Some(actions),
                                        false,
                                    )
                                    .await;

                                if result != DialogResult::Ok {
                                    return;
                                }

                                let filename = filename_value.borrow().clone().trim().to_string();
                                let priority =
                                    match parse_optional_priority_input(Some(priority_value.borrow().clone())) {
                                        Ok(priority) => priority,
                                        Err(err) => {
                                            services.toastr.error(err);
                                            return;
                                        }
                                    };

                                if filename.is_empty() {
                                    services.toastr.error(translate_clone.t("MESSAGES.DOWNLOAD.FAIL"));
                                    return;
                                }

                                let input_name = normalize_input_name(&selected.input_name);
                                match services
                                    .downloads
                                    .queue_download(resolved_url, filename, input_name, priority)
                                    .await
                                {
                                    Ok(_) => {
                                        services.toastr.success(translate_clone.t("MESSAGES.DOWNLOAD.DOWNLOAD_QUEUED"));
                                    }
                                    Err(_) => services.toastr.error(translate_clone.t("MESSAGES.DOWNLOAD.FAIL")),
                                }
                            });
                        }
                    }
                    ExplorerAction::Record => {
                        if !can_write_recordings {
                            popup_is_open_state.set(false);
                            return;
                        }
                        if let Some(dto) = &*selected_channel {
                            let dialog = dialog.clone();
                            let services = services.clone();
                            let translate_clone = translate_clone.clone();
                            let playlist_request = (*playlist_ctx.playlist_request).clone();
                            let selected = dto.clone();
                            let padding = Rc::clone(&recording_padding);
                            let target_name = match playlist_request.as_ref() {
                                Some(PlaylistRequest::Target(target_id)) => config.as_ref().and_then(|app_config| {
                                    target_name_for_id(&app_config.sources, *target_id, Some(&selected.input_name))
                                }),
                                _ => None,
                            };
                            spawn_local(async move {
                                if !ensure_recording_available(&services, &translate_clone).await {
                                    return;
                                }
                                let target_name = if let Some(name) = target_name {
                                    name
                                } else {
                                    services.toastr.error(translate_clone.t("MESSAGES.RECORDING.NO_TARGET"));
                                    return;
                                };
                                let source = RecordingSourceInput {
                                    target_id: target_name,
                                    virtual_id: selected.virtual_id.to_string(),
                                    cluster: selected.cluster,
                                    input_name: selected.input_name.clone(),
                                };
                                let now = chrono::Utc::now().timestamp();
                                let program_end = now + 90 * 60;
                                let prefill = RecordingFormPrefill::new(
                                    source,
                                    selected.title.clone(),
                                    now,
                                    program_end,
                                    (*padding).clone(),
                                )
                                .with_channel_name(selected.title.clone());
                                let request_slot: Rc<RefCell<Option<CreateRecordingTaskRequest>>> =
                                    Rc::new(RefCell::new(None));
                                let on_submit = {
                                    let request_slot = Rc::clone(&request_slot);
                                    Callback::from(move |request: CreateRecordingTaskRequest| {
                                        *request_slot.borrow_mut() = Some(request);
                                    })
                                };
                                let on_cancel = Callback::from(|()| {});
                                let body = html! {
                                    <RecordingForm
                                        prefill={prefill}
                                        has_recording_write={can_write_recordings}
                                        is_admin_role={is_admin_role}
                                        on_submit={on_submit}
                                        on_cancel={on_cancel}
                                    />
                                };
                                let actions = DialogActions {
                                    left: Some(vec![DialogAction::new(
                                        "cancel",
                                        "LABEL.CANCEL",
                                        DialogResult::Cancel,
                                        Some("Close".to_owned()),
                                        None,
                                    )]),
                                    right: vec![DialogAction::new_focused(
                                        "record",
                                        "LABEL.RECORD",
                                        DialogResult::Ok,
                                        Some("Record".to_owned()),
                                        Some("primary".to_string()),
                                    )],
                                };
                                let result = dialog.content(body, Some(actions), false).await;
                                if result != DialogResult::Ok {
                                    return;
                                }
                                let request = if let Some(r) = request_slot.borrow_mut().take() {
                                    r
                                } else {
                                    services.toastr.error(translate_clone.t("MESSAGES.RECORDING.NO_REQUEST"));
                                    return;
                                };
                                let recording_svc = RecordingService::new();
                                match recording_svc.create_task(request).await {
                                    Ok(_) => {
                                        services.toastr.success(translate_clone.t("MESSAGES.RECORDING.QUEUED"));
                                    }
                                    Err(err) => {
                                        services.toastr.error(err.to_string());
                                    }
                                }
                            });
                        }
                    }
                }
            }
            popup_is_open_state.set(false);
        })
    };

    let handle_back_click = {
        let current_item = current_item.clone();
        Callback::from(move |_| match *current_item {
            ExplorerLevel::Categories => {}
            ExplorerLevel::Group(_) => {
                current_item.set(ExplorerLevel::Categories);
            }
            ExplorerLevel::SeriesFolder(ref group, _) => {
                current_item.set(ExplorerLevel::Group(group.clone()));
            }
            ExplorerLevel::SeriesInfo(ref group, _, _) => {
                current_item.set(ExplorerLevel::Group(group.clone()));
            }
            ExplorerLevel::Favorites => {
                current_item.set(ExplorerLevel::Categories);
            }
        })
    };

    let handle_search = {
        let services = service_ctx.clone();
        let set_playlist = playlist.clone();
        let set_current_item = current_item.clone();
        let context = context.clone();
        Callback::from(move |search_req| match search_req {
            SearchRequest::Clear => set_playlist.set((*context.playlist).clone()),
            SearchRequest::Text(ref _text, ref _search_fields)
            | SearchRequest::Regexp(ref _text, ref _search_fields) => {
                services.event.broadcast(EventMessage::Busy(BusyStatus::Show));
                let set_playlist = set_playlist.clone();
                let set_current_item = set_current_item.clone();
                let context = context.clone();
                let services = services.clone();
                spawn_local(async move {
                    let filtered =
                        context.playlist.as_ref().and_then(|categories| categories.filter(&search_req)).map(Rc::new);
                    set_playlist.set(filtered);
                    set_current_item.set(ExplorerLevel::Categories);
                    services.event.broadcast(EventMessage::Busy(BusyStatus::Hide));
                });
            }
        })
    };

    let handle_category_select = {
        let set_current_item = current_item.clone();
        Callback::from(move |(group, _event): (Rc<UiPlaylistGroup>, MouseEvent)| {
            set_current_item.set(ExplorerLevel::Group(group));
        })
    };

    let render_cluster = |cluster: XtreamCluster, list: &Vec<Rc<UiPlaylistGroup>>| {
        list.iter()
            .map(|group| {
                let group_clone = group.clone();
                let on_click = {
                    let category_select = handle_category_select.clone();
                    Callback::from(move |event: MouseEvent| {
                        category_select.emit((group_clone.clone(), event));
                    })
                };
                html! {
                <span class={format!("tp__playlist-explorer__item tp__playlist-explorer__item-{}", cluster.to_string().to_lowercase())} onclick={on_click}>
                    { group.title.clone() }
                </span>
            }
            })
            .collect::<Html>()
    };

    let render_cluster_panel = |cluster: XtreamCluster, list: Option<&Vec<Rc<UiPlaylistGroup>>>| match list {
        Some(list) if !list.is_empty() => render_cluster(cluster, list),
        _ => html! { <NoContent text={translate.t("LABEL.NO_CONTENT")} /> },
    };

    let render_categories = || {
        if playlist.is_none() {
            html! {
                <NoContent
                    text={translate.t("MESSAGES.PLAYLIST_EXPLORER.SELECT_A_PLAYLIST_TO_VIEW_CONTENT")}
                    hint={translate.t("MESSAGES.PLAYLIST_EXPLORER.SELECT_A_PLAYLIST_HINT")}
                />
            }
        } else {
            let active_cluster = cluster_visible.intern();
            html! {
            <div class="tp__playlist-explorer__categories">
                <div class="tp__playlist-explorer__categories-sidebar tp__app-sidebar__content">
                    <IconButton class={format!("tp__app-sidebar-menu--{}{}", XtreamCluster::Live, if *cluster_visible == XtreamCluster::Live { " active" } else {""})}  icon="Live" name={XtreamCluster::Live.to_string()} onclick={&handle_cluster_change}></IconButton>
                    <IconButton class={format!("tp__app-sidebar-menu--{}{}", XtreamCluster::Video, if *cluster_visible == XtreamCluster::Video { " active" } else {""})} icon="Video" name={XtreamCluster::Video.to_string()} onclick={&handle_cluster_change}></IconButton>
                    <IconButton class={format!("tp__app-sidebar-menu--{}{}", XtreamCluster::Series, if *cluster_visible == XtreamCluster::Series { " active" } else {""})} icon="Series" name={XtreamCluster::Series.to_string()} onclick={&handle_cluster_change}></IconButton>
                </div>
                <div class="tp__playlist-explorer__categories-content">
                    <Panel class="tp__full-width" value={XtreamCluster::Live.intern()} active={active_cluster.clone()}>
                        <div class="tp__playlist-explorer__categories-list">
                            { render_cluster_panel(XtreamCluster::Live, playlist.as_ref().and_then(|response| response.live.as_ref())) }
                            </div>
                    </Panel>
                    <Panel class="tp__full-width" value={XtreamCluster::Video.intern()} active={active_cluster.clone()}>
                        <div class="tp__playlist-explorer__categories-list">
                            { render_cluster_panel(XtreamCluster::Video, playlist.as_ref().and_then(|response| response.vod.as_ref())) }
                            </div>
                    </Panel>
                    <Panel class="tp__full-width" value={XtreamCluster::Series.intern()} active={active_cluster}>
                        <div class="tp__playlist-explorer__categories-list">
                            { render_cluster_panel(XtreamCluster::Series, playlist.as_ref().and_then(|response| response.series.as_ref())) }
                        </div>
                    </Panel>
                </div>
            </div>
            }
        }
    };

    let render_channel_logo = |logo: &str, title: &str| {
        let logo = if logo.is_empty() { "assets/missing-logo.svg".to_string() } else { logo.to_string() };
        let alt = if title.is_empty() { translate.t("LABEL.CHANNEL") } else { title.to_string() };
        html! {
            <span  class="tp__playlist-explorer__channel-logo">
                <img  alt={alt} src={logo} loading="lazy"
                onerror={Callback::from(move |e: web_sys::Event| {
                if let Some(target)  = e.target() {
                    if let Ok(img) = target.dyn_into::<web_sys::HtmlImageElement>() {
                        img.set_src("assets/missing-logo.svg");
                    }
                }
                })}/>
            </span>
        }
    };

    let render_series_favorite_button = |title: &str| {
        let key = series_favorite_key(title);
        let is_favorite = favorite_series.contains(&key);
        let label_key = if is_favorite {
            "LABEL.REMOVE_SERIES_FROM_FAVORITES"
        } else {
            "LABEL.ADD_SERIES_TO_FAVORITES"
        };
        let label = format!("{}: {title}", translate.t(label_key));
        let favorite_title = key.clone();
        let toggle_favorite = handle_toggle_series_favorite.clone();
        let onclick = Callback::from(move |(_name, event): (String, MouseEvent)| {
            event.prevent_default();
            event.stop_propagation();
            toggle_favorite.emit(favorite_title.clone());
        });
        html! {
            <IconButton
                class={if is_favorite { "tp__playlist-explorer__favorite-button is-favorite" } else { "tp__playlist-explorer__favorite-button" }}
                name={key}
                icon={if is_favorite { "Star" } else { "StarBorder" }}
                hint={label.clone()}
                aria_label={Some(label)}
                aria_pressed={Some(is_favorite)}
                onclick={onclick}
            />
        }
    };

    let render_item_favorite_button = |item: &UiPlaylistItem| {
        let key = playlist_item_favorite_key(item);
        let is_favorite = favorite_items.contains(&key);
        let label_key = if is_favorite {
            "LABEL.REMOVE_CONTENT_FROM_FAVORITES"
        } else {
            "LABEL.ADD_CONTENT_TO_FAVORITES"
        };
        let label = format!("{}: {}", translate.t(label_key), item.title);
        let favorite_key = key.clone();
        let toggle_favorite = handle_toggle_item_favorite.clone();
        let onclick = Callback::from(move |(_name, event): (String, MouseEvent)| {
            event.prevent_default();
            event.stop_propagation();
            toggle_favorite.emit(favorite_key.clone());
        });
        html! {
            <IconButton
                class={if is_favorite { "tp__playlist-explorer__favorite-button is-favorite" } else { "tp__playlist-explorer__favorite-button" }}
                name={key}
                icon={if is_favorite { "Star" } else { "StarBorder" }}
                hint={label.clone()}
                aria_label={Some(label)}
                aria_pressed={Some(is_favorite)}
                onclick={onclick}
            />
        }
    };

    let render_live = |chan: &Rc<UiPlaylistItem>| {
        let popup_onclick = handle_popup_onclick.clone();
        let chan_clone = Rc::clone(chan);
        html! {
        <span class="tp__playlist-explorer__channel tp__playlist-explorer__channel-live">
            <button class="tp__icon-button" onclick={Callback::from(move |event: MouseEvent| popup_onclick.emit((chan_clone.clone(), event)))}>
                <AppIcon name="Popup"></AppIcon>
            </button>
            {render_channel_logo(&chan.logo, &chan.title)}
            <span class="tp__playlist-explorer__channel-title">{chan.title.clone()}</span>
            {render_item_favorite_button(chan)}
            </span>
        }
    };

    let render_movie = |chan: &Rc<UiPlaylistItem>| {
        let popup_onclick = handle_popup_onclick.clone();
        let chan_clone = Rc::clone(chan);
        html! {
            <span class="tp__playlist-explorer__channel tp__playlist-explorer__channel-video">
                {render_channel_logo(&chan.logo, &chan.title)}
                {
                    html_if!(chan.rating > 0.001, {
                        <Chip class="tp__playlist-explorer__channel-video-rating" label={format_float_localized(chan.rating, 1, false)} />
                    })
                }
                <span class="tp__playlist-explorer__channel-video-info">
                    <button class="tp__icon-button" onclick={Callback::from(move |event: MouseEvent| popup_onclick.emit((chan_clone.clone(), event)))}>
                        <AppIcon name="Popup"></AppIcon>
                    </button>
                    <span class="tp__playlist-explorer__channel-video-title">{chan.title.clone()}</span>
                </span>
                {render_item_favorite_button(chan)}
            </span>
        }
    };

    let render_series = |group: &Rc<UiPlaylistGroup>, chan: &Rc<UiPlaylistItem>| {
        let popup_onclick = handle_popup_onclick.clone();
        let chan_clone = Rc::clone(chan);
        let chan_click = {
            let chan_clone = chan.clone();
            let group = group.clone();
            let load_series_info = load_series_info.clone();
            Callback::from(move |event: MouseEvent| {
                event.prevent_default();
                event.stop_propagation();
                load_series_info(group.clone(), chan_clone.clone());
            })
        };
        html! {
            <span onclick={chan_click} class="tp__playlist-explorer__channel tp__playlist-explorer__channel-series">
                {render_channel_logo(&chan.logo, &chan.title)}
                {
                    html_if!(chan.rating > 0.001, {
                        <Chip class="tp__playlist-explorer__channel-series-rating" label={format_float_localized(chan.rating, 1, false)} />
                    })
                }
                {render_series_favorite_button(&chan.title)}
                <span class="tp__playlist-explorer__channel-series-info">
                    <button class="tp__icon-button" onclick={Callback::from(move |event: MouseEvent| popup_onclick.emit((chan_clone.clone(), event)))}>
                        <AppIcon name="Popup"></AppIcon>
                    </button>
                    <span class="tp__playlist-explorer__channel-series-title">{chan.title.clone()}</span>
                </span>
            </span>
        }
    };

    let render_series_folder_card = |group: &Rc<UiPlaylistGroup>, folder: Rc<SeriesFolder>| {
        let folder_title = folder.title.clone();
        let folder_logo = folder.logo.clone();
        let group_for_click = group.clone();
        let folder_for_click = folder.clone();
        let on_click = {
            let current_item = current_item.clone();
            Callback::from(move |event: MouseEvent| {
                event.prevent_default();
                event.stop_propagation();
                current_item.set(ExplorerLevel::SeriesFolder(group_for_click.clone(), folder_for_click.clone()));
            })
        };

        html! {
            <span key={format!("series-folder-{folder_title}")} onclick={on_click} class="tp__playlist-explorer__channel tp__playlist-explorer__channel-series">
                {render_channel_logo(&folder_logo, &folder_title)}
                {render_series_favorite_button(&folder_title)}
                <span class="tp__playlist-explorer__channel-series-info">
                    <span class="tp__playlist-explorer__channel-series-title">{folder_title}</span>
                </span>
            </span>
        }
    };

    let render_episode = |chan: &SeriesStreamDetailEpisodeProperties,
                          series_episodes: Option<Rc<Vec<BrowserPlayerEpisode>>>| {
        let channel_select = ChannelSelection {
            virtual_id: VirtualId::new(chan.id),
            provider_id: String::new(),
            cluster: XtreamCluster::Series,
            downloadable: true,
            // Falls back to the episode fetch path in the menu handler when empty
            url: chan.direct_source.to_string(),
            title: chan.title.to_string(),
            input_name: String::new(),
            series_episodes,
        };
        let popup_onclick = handle_episode_popup_onclick.clone();
        let rating = chan.rating.unwrap_or_default();
        html! {
            <span class="tp__playlist-explorer__channel tp__playlist-explorer__channel-episode">
                {render_channel_logo(&chan.movie_image, &chan.title)}
                {
                    html_if!(rating > 0.001, {
                        <Chip class="tp__playlist-explorer__channel-episode-rating" label={format_float_localized(rating, 1, false)} />
                    })
                }
                <span class="tp__playlist-explorer__channel-episode-info">
                    <button class="tp__icon-button" onclick={Callback::from(move |event: MouseEvent| popup_onclick.emit((channel_select.clone(), event)))}>
                        <AppIcon name="Popup"></AppIcon>
                    </button>
                    <span class="tp__playlist-explorer__channel-episode-title">{chan.title.clone()}</span>
                </span>
            </span>
        }
    };

    let render_series_folder = |folder: &Rc<SeriesFolder>| {
        let player_episodes = Rc::new(
            folder
                .seasons
                .iter()
                .flat_map(|(season, episodes)| {
                    episodes.iter().map(|(episode, channel)| BrowserPlayerEpisode {
                        virtual_id: channel.virtual_id,
                        title: channel.title.to_string(),
                        label: series_episode_display_title(&channel.title, &folder.title),
                        url: channel.url.to_string(),
                        input_name: channel.input_name.to_string(),
                        season: *season,
                        episode: *episode,
                    })
                })
                .collect::<Vec<_>>(),
        );
        let style = if folder.logo.is_empty() {
            String::new()
        } else {
            format!("background-image: url(\"{}\");", folder.logo)
        };
        let seasons_html = folder
            .seasons
            .iter()
            .map(|(season, episodes)| {
                let episodes_html = episodes
                    .iter()
                    .map(|(episode_number, chan)| {
                        let selected = ChannelSelection {
                            virtual_id: VirtualId::new(chan.virtual_id),
                            provider_id: chan.provider_id.to_string(),
                            cluster: XtreamCluster::Series,
                            downloadable: true,
                            url: chan.url.to_string(),
                            title: chan.title.to_string(),
                            input_name: chan.input_name.to_string(),
                            series_episodes: Some(player_episodes.clone()),
                        };
                        let popup_onclick = handle_episode_popup_onclick.clone();
                        let episode_title = series_episode_display_title(&chan.title, &folder.title);
                        let episode_key = format!("episode-{}-{episode_number}", chan.virtual_id);
                        html! {
                            <span key={episode_key} class="tp__playlist-explorer__channel tp__playlist-explorer__channel-episode">
                                {render_channel_logo(&chan.logo, &chan.title)}
                                {
                                    html_if!(chan.rating > 0.001, {
                                        <Chip class="tp__playlist-explorer__channel-episode-rating" label={format_float_localized(chan.rating, 1, false)} />
                                    })
                                }
                                <span class="tp__playlist-explorer__channel-episode-info">
                                    <button class="tp__icon-button" onclick={Callback::from(move |event: MouseEvent| popup_onclick.emit((selected.clone(), event)))}>
                                        <AppIcon name="Popup"></AppIcon>
                                    </button>
                                    <span class="tp__playlist-explorer__channel-episode-title">{episode_title}</span>
                                </span>
                            </span>
                        }
                    })
                    .collect::<Html>();
                html! {
                    <div key={format!("season-{season}")}>
                        <div class="tp__playlist-explorer__series-info__season">
                            <span class="tp__playlist-explorer__series-info__season-title">
                                {translate.t("LABEL.SEASON")}{" - "}{season}
                            </span>
                        </div>
                        <div class="tp__playlist-explorer__group-list tp__playlist-explorer__group-list-episodes">
                            {episodes_html}
                        </div>
                    </div>
                }
            })
            .collect::<Html>();

        html! {
            <div class="tp__playlist-explorer__series-info">
                <div class="tp__playlist-explorer__series-info__header">
                    <div class="tp__playlist-explorer__series-info__body-top" style={style}>
                        <div class="tp__playlist-explorer__series-info__body-top-backdrop"></div>
                        {render_series_favorite_button(&folder.title)}
                        <div class="tp__playlist-explorer__series-info__body-top-content">
                            <span class="tp__playlist-explorer__series-info__title">{folder.title.clone()}</span>
                        </div>
                    </div>
                </div>
                <div class="tp__playlist-explorer__series-info__body">
                    {seasons_html}
                </div>
            </div>
        }
    };

    let render_channel = |group: &Rc<UiPlaylistGroup>, chan: &Rc<UiPlaylistItem>| match chan.xtream_cluster {
        XtreamCluster::Live => render_live(chan),
        XtreamCluster::Video => render_movie(chan),
        XtreamCluster::Series => render_series(group, chan),
    };

    let active_sort = *playlist_sort;
    let render_group = |group: &Rc<UiPlaylistGroup>| {
        let channels = if group.xtream_cluster == XtreamCluster::Series {
            let mut entries = build_series_entries(&group.channels);
            sort_by_rating(&mut entries, active_sort, series_entry_rating);
            entries
                .into_iter()
                .map(|entry| match entry {
                    SeriesExplorerEntry::Item(channel) => render_series(group, &channel),
                    SeriesExplorerEntry::Folder(folder) => render_series_folder_card(group, Rc::new(folder)),
                })
                .collect::<Html>()
        } else {
            let mut ordered_channels = group.channels.clone();
            sort_by_rating(&mut ordered_channels, active_sort, |channel| channel.rating);
            ordered_channels.iter().map(|channel| render_channel(group, channel)).collect::<Html>()
        };

        html! {
            <div class="tp__playlist-explorer__group">
              <div class={format!("tp__playlist-explorer__group-list tp__playlist-explorer__group-list-{}", group.xtream_cluster.to_string().to_lowercase())}>
              {
                  if group.channels.is_empty() {
                      html! { <NoContent text={translate.t("LABEL.NO_CONTENT")} /> }
                  } else {
                      channels
                  }
              }
              </div>
            </div>
        }
    };

    let render_favorites = || {
        let mut favorite_sections = Vec::new();
        let mut rendered_items = HashSet::new();

        let mut live_items = Vec::new();
        if let Some(groups) = playlist.as_ref().and_then(|categories| categories.live.as_ref()) {
            for group in groups {
                for channel in &group.channels {
                    let key = playlist_item_favorite_key(channel);
                    if favorite_items.contains(&key) && rendered_items.insert(key) {
                        live_items.push(channel.clone());
                    }
                }
            }
        }
        sort_by_rating(&mut live_items, active_sort, |channel| channel.rating);
        let live_cards = live_items.iter().map(render_live).collect::<Vec<_>>();
        if !live_cards.is_empty() {
            favorite_sections.push(html! {
                <div class="tp__playlist-explorer__favorites-section">
                    <h3 class="tp__playlist-explorer__favorites-title">{translate.t("LABEL.FAVORITE_CHANNELS")}</h3>
                    <div class="tp__playlist-explorer__group-list tp__playlist-explorer__group-list-live">
                        {for live_cards}
                    </div>
                </div>
            });
        }

        let mut favorite_movies = Vec::new();
        if let Some(groups) = playlist.as_ref().and_then(|categories| categories.vod.as_ref()) {
            for group in groups {
                for movie in &group.channels {
                    let key = playlist_item_favorite_key(movie);
                    if favorite_items.contains(&key) && rendered_items.insert(key) {
                        favorite_movies.push(movie.clone());
                    }
                }
            }
        }
        sort_by_rating(&mut favorite_movies, active_sort, |movie| movie.rating);
        let movie_cards = favorite_movies.iter().map(render_movie).collect::<Vec<_>>();
        if !movie_cards.is_empty() {
            favorite_sections.push(html! {
                <div class="tp__playlist-explorer__favorites-section">
                    <h3 class="tp__playlist-explorer__favorites-title">{translate.t("LABEL.FAVORITE_MOVIES")}</h3>
                    <div class="tp__playlist-explorer__group-list tp__playlist-explorer__group-list-video">
                        {for movie_cards}
                    </div>
                </div>
            });
        }

        let mut favorite_series_entries = Vec::new();
        let mut rendered_series = HashSet::new();
        if let Some(groups) = playlist.as_ref().and_then(|categories| categories.series.as_ref()) {
            for group in groups {
                for entry in build_series_entries(&group.channels) {
                    match entry {
                        SeriesExplorerEntry::Item(channel) => {
                            let key = series_favorite_key(&channel.title);
                            if favorite_series.contains(&key) && rendered_series.insert(key) {
                                favorite_series_entries.push((group.clone(), SeriesExplorerEntry::Item(channel)));
                            }
                        }
                        SeriesExplorerEntry::Folder(folder) => {
                            let key = series_favorite_key(&folder.title);
                            if favorite_series.contains(&key) && rendered_series.insert(key) {
                                favorite_series_entries
                                    .push((group.clone(), SeriesExplorerEntry::Folder(folder)));
                            }
                        }
                    }
                }
            }
        }
        sort_by_rating(&mut favorite_series_entries, active_sort, |(_, entry)| series_entry_rating(entry));
        let series_cards = favorite_series_entries
            .iter()
            .map(|(group, entry)| match entry {
                SeriesExplorerEntry::Item(channel) => render_series(group, channel),
                SeriesExplorerEntry::Folder(folder) => render_series_folder_card(group, Rc::new(folder.clone())),
            })
            .collect::<Vec<_>>();
        if !series_cards.is_empty() {
            favorite_sections.push(html! {
                <div class="tp__playlist-explorer__favorites-section">
                    <h3 class="tp__playlist-explorer__favorites-title">{translate.t("LABEL.FAVORITE_SERIES")}</h3>
                    <div class="tp__playlist-explorer__group-list tp__playlist-explorer__group-list-series">
                        {for series_cards}
                    </div>
                </div>
            });
        }

        if favorite_sections.is_empty() {
            html! {
                <NoContent
                    text={translate.t("LABEL.NO_FAVORITES")}
                    hint={translate.t("LABEL.FAVORITES_HINT")}
                />
            }
        } else {
            html! { <div class="tp__playlist-explorer__favorites">{for favorite_sections}</div> }
        }
    };

    let render_series_info = |series_info: &Rc<UiPlaylistItem>, props: Option<&Box<SeriesStreamProperties>>| {
        // UiPlaylistItem has no additional_properties - props are passed in or None
        let series_info_props = props;
        let (mut backdrop, plot, cast, genre, release_date, rating, details) = match series_info_props {
            Some(series_props) => {
                let backdrop = series_props.backdrop_path.as_ref().and_then(|l| l.first()).map_or_else(
                    || {
                        if series_props.cover.is_empty() {
                            series_info.logo.to_string()
                        } else {
                            series_props.cover.to_string()
                        }
                    },
                    ToString::to_string,
                );
                (
                    Some(backdrop.clone()),
                    series_props.plot.as_deref().map(ToString::to_string).unwrap_or_default(),
                    series_props.cast.to_string(),
                    series_props.genre.as_deref().map(ToString::to_string).unwrap_or_default(),
                    series_props.release_date.as_deref().map(ToString::to_string).unwrap_or_default(),
                    series_props.rating,
                    series_props.details.as_ref(),
                )
            }
            _ => (None, String::new(), String::new(), String::new(), String::new(), 0.0, None),
        };

        if !series_info.logo.is_empty() && backdrop.as_ref().is_none_or(std::string::String::is_empty) {
            backdrop = Some(series_info.logo.to_string());
        }

        let style = backdrop.as_ref().map(|b| format!("background-image: url(\"{b}\");")).unwrap_or_default();

        let series_html = html! {
            <div class="tp__playlist-explorer__series-info__body-top" style={style}>
                <div class="tp__playlist-explorer__series-info__body-top-backdrop"></div>
                {render_series_favorite_button(&series_info.title)}
                <div class="tp__playlist-explorer__series-info__body-top-content">
                    <span class="tp__playlist-explorer__series-info__title">{series_info.title.clone()}</span>
                    <span class="tp__playlist-explorer__series-info__infos">
                        {
                            html_if!(rating > 0.001, {
                            <>
                             <span class="tp__playlist-explorer__series-info__nowrap">
                                 <Chip class="tp__playlist-explorer__series-info__rating" label={format_float_localized(rating, 1, false)} />
                            </span>
                            {"◦"}
                            </>
                        })}
                        <span class="tp__playlist-explorer__series-info__nowrap">{release_date}</span>
                        {"◦"}
                        <span>{genre}</span>
                    </span>
                    <span class="tp__playlist-explorer__series-info__plot">{plot}</span>
                    <span class="tp__playlist-explorer__series-info__cast">{cast}</span>
                </div>
            </div>
        };

        let player_episodes = details.and_then(|d| d.episodes.as_ref()).map(|episodes| {
            let mut player_episodes = episodes
                .iter()
                .map(|episode| BrowserPlayerEpisode {
                    virtual_id: episode.id,
                    title: episode.title.to_string(),
                    label: series_episode_display_title(&episode.title, &series_info.title),
                    url: episode.direct_source.to_string(),
                    input_name: series_info.input_name.to_string(),
                    season: episode.season,
                    episode: episode.episode_num,
                })
                .collect::<Vec<_>>();
            player_episodes.sort_by_key(|episode| (episode.season, episode.episode, episode.virtual_id));
            Rc::new(player_episodes)
        });

        let episodes_html = if let Some(episodes) = details.as_ref().and_then(|d| d.episodes.as_ref()) {
            let mut grouped: HashMap<u32, Vec<&SeriesStreamDetailEpisodeProperties>> = HashMap::new();
            for item in episodes {
                grouped.entry(item.season).or_default().push(item);
            }
            for season_episodes in grouped.values_mut() {
                season_episodes.sort_by_key(|episode| (episode.episode_num, episode.id));
            }
            let mut grouped_list: Vec<(u32, Vec<&SeriesStreamDetailEpisodeProperties>)> = grouped.into_iter().collect();
            grouped_list.sort_by_key(|(season, _)| *season);

            html! {
                for (season, season_episodes) in grouped_list.iter() {
                    <div key={format!("season-{season}")}>
                        <div class={"tp__playlist-explorer__series-info__season"}>
                            <span class={"tp__playlist-explorer__series-info__season-title"}>{translate.t("LABEL.SEASON")} {" - "} {season}</span>
                        </div>
                        <div class={"tp__playlist-explorer__group-list tp__playlist-explorer__group-list-episodes"}>
                            for episode in season_episodes.iter() {
                                { render_episode(episode, player_episodes.clone()) }
                            }
                        </div>
                    </div>
                }
            }
        } else {
            Html::default()
        };

        html! {
        <div class="tp__playlist-explorer__series-info">
            <div class="tp__playlist-explorer__series-info__header">
                { series_html }
            </div>
             <div class="tp__playlist-explorer__series-info__body">
                 {episodes_html}
            </div>
        </div>
        }
    };

    let is_favorites_view = matches!(&*current_item, ExplorerLevel::Favorites);
    let favorites_toggle_label = if is_favorites_view {
        translate.t("LABEL.SHOW_ALL_CONTENT")
    } else {
        translate.t("LABEL.SHOW_FAVORITES")
    };

    html! {
      <div class="tp__playlist-explorer">
        <div class="tp__playlist-explorer__header">
            <div class="tp__playlist-explorer__header-toolbar">
                <div class="tp__playlist-explorer__header-toolbar-actions">
                   <IconButton class={if matches!(*current_item, ExplorerLevel::Categories) { "disabled" } else {""}} name="back" icon="Back" onclick={handle_back_click} />
                   <IconButton
                        class={if is_favorites_view { "tp__playlist-explorer__favorites-toggle active" } else { "tp__playlist-explorer__favorites-toggle" }}
                        name="toggle-favorites"
                        icon={if is_favorites_view { "Star" } else { "StarBorder" }}
                        hint={favorites_toggle_label.clone()}
                        aria_label={Some(favorites_toggle_label)}
                        aria_pressed={Some(is_favorites_view)}
                        onclick={handle_favorites_toggle.clone()}
                    />
                  {
                    match *current_item {
                        ExplorerLevel::Categories => html!{} ,
                        ExplorerLevel::Group(ref group) => html!{ <span>{group.title.to_string()}</span> },
                        ExplorerLevel::SeriesFolder(_, ref folder) => html!{ <span>{folder.title.clone()}</span> },
                        ExplorerLevel::SeriesInfo(_, ref pli, _) => html!{ <span>{pli.title.to_string()}</span> },
                        ExplorerLevel::Favorites => html!{ <span>{translate.t("LABEL.FAVORITES")}</span> },
                    }
                  }
                </div>
                <div class="tp__playlist-explorer__header-toolbar-search">
                  <DropDownIconButton
                    name="sort-rating"
                    icon="SortDesc"
                    class={if active_sort == PlaylistExplorerSort::RatingDescending { "option-active" } else { "" }}
                    aria_label={translate.t("LABEL.SORT")}
                    options={sort_options.clone()}
                    on_select={handle_sort_change.clone()}
                  />
                  <Search onsearch={handle_search} options={search_fields.clone()} on_fields_change={handle_search_fields_change}/>
                </div>
            </div>
        </div>
        <div class="tp__playlist-explorer__body">
          {
            match *current_item {
                ExplorerLevel::Categories => html!{render_categories()} ,
                ExplorerLevel::Group(ref group) => html!{ render_group(group) },
                ExplorerLevel::SeriesFolder(_, ref folder) => html!{ render_series_folder(folder) },
                ExplorerLevel::SeriesInfo(_, ref pli, ref props) => html!{ render_series_info(pli, props.as_ref()) },
                ExplorerLevel::Favorites => html!{ render_favorites() },
            }
          }
        </div>

        <PopupMenu is_open={*popup_is_open} anchor_ref={(*popup_anchor_ref).clone()} on_close={handle_popup_close}>
            { html_if!(context.playlist_request.as_ref().is_some_and(|r| matches!(r, PlaylistRequest::Target(_))), {
                <>
                 <MenuItem icon="Clipboard" name={ExplorerAction::CopyLinkTuliproxVirtualId.to_string()} label={translate.t("LABEL.COPY_LINK_TULIPROX_VIRTUAL_ID")} onclick={&handle_menu_click}></MenuItem>
                 <MenuItem icon="Clipboard" name={ExplorerAction::CopyLinkTuliproxWebPlayerUrl.to_string()} label={translate.t("LABEL.COPY_LINK_TULIPROX_WEBPLAYER_URL")} onclick={&handle_menu_click}></MenuItem>
                </>
             })
            }
            { html_if!(can_show_play_action(selected_channel.as_ref()), {
                <MenuItem icon="PlayArrow" name={ExplorerAction::PlayInBrowser.to_string()} label={translate.t("LABEL.PLAY_IN_BROWSER")} onclick={&handle_menu_click}></MenuItem>
            })}
            <MenuItem icon="Clipboard" name={ExplorerAction::CopyLinkProviderUrl.to_string()} label={translate.t("LABEL.COPY_LINK_PROVIDER_URL")} onclick={&handle_menu_click}></MenuItem>
            { html_if!(
                can_show_record_action(can_write_recordings, selected_channel.as_ref()),
                {
                <MenuItem icon="Record" name={ExplorerAction::Record.to_string()} label={translate.t("LABEL.RECORD")} onclick={&handle_menu_click}></MenuItem>
            })}
            { html_if!(
                can_show_download_action(can_write_downloads, selected_channel.as_ref()),
                {
                <MenuItem icon="Download" name={ExplorerAction::Download.to_string()} label={translate.t("LABEL.DOWNLOAD")} onclick={&handle_menu_click}></MenuItem>
            })}
        </PopupMenu>
      </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_download_filename, can_show_download_action, can_show_record_action, compare_rating_desc,
        normalize_input_name, parse_optional_priority_input, sort_by_rating, ChannelSelection,
        PlaylistExplorerSort,
    };
    use shared::model::{VirtualId, XtreamCluster};

    #[test]
    fn parse_optional_priority_input_treats_blank_as_none() {
        assert_eq!(parse_optional_priority_input(None), Ok(None));
        assert_eq!(parse_optional_priority_input(Some(String::new())), Ok(None));
        assert_eq!(parse_optional_priority_input(Some("   ".to_string())), Ok(None));
    }

    #[test]
    fn parse_optional_priority_input_parses_valid_i8_values() {
        assert_eq!(parse_optional_priority_input(Some("-1".to_string())), Ok(Some(-1)));
        assert_eq!(parse_optional_priority_input(Some("12".to_string())), Ok(Some(12)));
        assert_eq!(parse_optional_priority_input(Some(" 0 ".to_string())), Ok(Some(0)));
    }

    #[test]
    fn parse_optional_priority_input_rejects_invalid_non_empty_values() {
        assert!(parse_optional_priority_input(Some("abc".to_string())).is_err());
    }

    #[test]
    fn normalize_input_name_treats_blank_as_none() {
        assert_eq!(normalize_input_name(""), None);
        assert_eq!(normalize_input_name("   "), None);
        assert_eq!(normalize_input_name(" provider-a "), Some("provider-a".to_string()));
    }

    #[test]
    fn popup_actions_require_download_write_permission() {
        let live = ChannelSelection {
            virtual_id: VirtualId::default(),
            provider_id: String::new(),
            cluster: XtreamCluster::Live,
            downloadable: false,
            url: String::new(),
            title: "Live".to_string(),
            input_name: String::new(),
            series_episodes: None,
        };
        let vod = ChannelSelection {
            virtual_id: VirtualId::default(),
            provider_id: String::new(),
            cluster: XtreamCluster::Video,
            downloadable: true,
            url: String::new(),
            title: "VOD".to_string(),
            input_name: String::new(),
            series_episodes: None,
        };
        let series_container = ChannelSelection {
            virtual_id: VirtualId::default(),
            provider_id: String::new(),
            cluster: XtreamCluster::Series,
            downloadable: false,
            url: String::new(),
            title: "Series".to_string(),
            input_name: String::new(),
            series_episodes: None,
        };
        let episode = ChannelSelection {
            virtual_id: VirtualId::default(),
            provider_id: String::new(),
            cluster: XtreamCluster::Series,
            downloadable: true,
            url: String::new(),
            title: "Episode".to_string(),
            input_name: String::new(),
            series_episodes: None,
        };

        assert!(!can_show_record_action(false, Some(&live)));
        assert!(!can_show_download_action(false, Some(&vod)));
        assert!(can_show_record_action(true, Some(&live)));
        assert!(can_show_download_action(true, Some(&vod)));
        assert!(!can_show_download_action(true, Some(&live)));
        assert!(!can_show_download_action(true, Some(&series_container)));
        assert!(can_show_download_action(true, Some(&episode)));
        assert!(!can_show_record_action(true, Some(&vod)));
    }

    #[test]
    fn build_download_filename_keeps_url_extension() {
        let filename = build_download_filename("My Movie", "https://example.com/video.mkv?token=1");
        assert_eq!(filename, "My_Movie.mkv");
    }

    #[test]
    fn build_download_filename_falls_back_to_mp4() {
        let filename = build_download_filename("Episode 01", "https://example.com/stream");
        assert_eq!(filename, "Episode_01.mp4");
    }

    #[test]
    fn rating_sort_places_highest_first_and_unrated_items_last() {
        let mut items = vec![("Unrated", 0.0), ("Lower", 5.5), ("Highest", 9.2), ("Middle", 7.0)];

        sort_by_rating(&mut items, PlaylistExplorerSort::RatingDescending, |item| item.1);

        assert_eq!(items.iter().map(|item| item.0).collect::<Vec<_>>(), vec!["Highest", "Middle", "Lower", "Unrated"]);
        assert_eq!(compare_rating_desc(9.2, 5.5), std::cmp::Ordering::Less);
        assert_eq!(compare_rating_desc(0.0, 5.5), std::cmp::Ordering::Greater);
    }

    #[test]
    fn provider_order_sort_preserves_the_original_sequence() {
        let mut items = vec![("Lower", 5.5), ("Highest", 9.2), ("Unrated", 0.0)];

        sort_by_rating(&mut items, PlaylistExplorerSort::ProviderOrder, |item| item.1);

        assert_eq!(items.iter().map(|item| item.0).collect::<Vec<_>>(), vec!["Lower", "Highest", "Unrated"]);
    }
}
