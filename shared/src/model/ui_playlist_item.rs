use crate::{
    model::{
        CommonPlaylistItem, M3uPlaylistItem, PlaylistItem, PlaylistItemType, StreamProperties, VirtualId,
        XtreamCluster, XtreamPlaylistItem,
    },
    utils::{arc_str_option_serde, arc_str_serde, Internable, CONSTANTS},
};
use serde_tuple::{Deserialize_tuple, Serialize_tuple};
use std::sync::Arc;

/// Lightweight playlist item for UI streaming.
#[derive(Debug, Clone, Serialize_tuple, Deserialize_tuple, PartialEq)]
pub struct UiPlaylistItem {
    #[serde(rename = "v")]
    pub virtual_id: u32,
    #[serde(rename = "p", with = "arc_str_serde")]
    pub provider_id: Arc<str>,
    #[serde(rename = "n", with = "arc_str_serde")]
    pub name: Arc<str>,
    #[serde(rename = "t", with = "arc_str_serde")]
    pub title: Arc<str>,
    #[serde(rename = "g", with = "arc_str_serde")]
    pub group: Arc<str>,
    #[serde(rename = "l", with = "arc_str_serde")]
    pub logo: Arc<str>,
    #[serde(rename = "u", with = "arc_str_serde")]
    pub url: Arc<str>,
    #[serde(rename = "t")]
    pub item_type: PlaylistItemType,
    #[serde(rename = "x")]
    pub xtream_cluster: XtreamCluster,
    #[serde(rename = "c")]
    pub category_id: u32,
    #[serde(rename = "s")]
    pub rating: f64,
    #[serde(rename = "i", with = "arc_str_serde")]
    pub input_name: Arc<str>,
    // EPG channel identifier for per-stream programme lookup. None when no EPG is configured.
    #[serde(rename = "e", default, skip_serializing_if = "Option::is_none", with = "arc_str_option_serde")]
    pub epg_channel_id: Option<Arc<str>>,
}

impl UiPlaylistItem {
    pub fn from_target_item(mut item: XtreamPlaylistItem, is_m3u: bool) -> Self {
        // Legacy M3U caches can contain one SeriesInfo per episode. The folder
        // builder needs the embedded episode ID and URL, not the container ID.
        if is_m3u && item.item_type == PlaylistItemType::SeriesInfo && item.url.is_empty() {
            if let Some(StreamProperties::Series(series)) = item.additional_properties.as_ref() {
                if let Some(episodes) = series.details.as_ref().and_then(|details| details.episodes.as_ref()) {
                    if let [episode] = episodes.as_slice() {
                        if episode.title == item.title
                            && CONSTANTS.re_episode_code.is_match(&item.title)
                            && episode.id != 0
                            && !episode.direct_source.is_empty()
                        {
                            item.virtual_id = VirtualId::new(episode.id);
                            item.item_type = PlaylistItemType::Series;
                            item.url = Arc::clone(&episode.direct_source);
                        }
                    }
                }
            }
        }
        Self::from(item)
    }
}

/// Helper to pick the best logo: prefer `logo` if non-empty, else `logo_small`
fn pick_logo(logo: &Arc<str>, logo_small: &Arc<str>, props: Option<&StreamProperties>) -> Arc<str> {
    if !logo.is_empty() {
        return Arc::clone(logo);
    }
    if !logo_small.is_empty() {
        return Arc::clone(logo_small);
    }

    props
        .and_then(|p| match p {
            StreamProperties::Video(v) => non_empty(&v.stream_icon).or_else(|| {
                v.details.as_ref().and_then(|d| {
                    non_empty_opt(d.movie_image.as_ref())
                        .or_else(|| non_empty_opt(d.cover_big.as_ref()))
                        .or_else(|| d.backdrop_path.as_ref().and_then(|b| non_empty(b.first()?)))
                })
            }),
            StreamProperties::Series(s) => {
                non_empty(&s.cover).or_else(|| s.backdrop_path.as_ref().and_then(|b| non_empty(b.first()?)))
            }
            _ => None,
        })
        .unwrap_or_else(|| "".intern())
}

fn non_empty(s: &Arc<str>) -> Option<Arc<str>> { (!s.is_empty()).then(|| Arc::clone(s)) }

fn non_empty_opt(s: Option<&Arc<str>>) -> Option<Arc<str>> { s.and_then(non_empty) }

/// Helper to get rating
#[inline]
fn get_rating(props: Option<&StreamProperties>) -> f64 {
    if let Some(p) = props {
        return match p {
            StreamProperties::Video(v) => v.rating.unwrap_or_default(),
            StreamProperties::Series(s) => s.rating,
            StreamProperties::Live(_) | StreamProperties::Episode(_) => 0.0,
        };
    }
    0.0
}

impl From<&CommonPlaylistItem> for UiPlaylistItem {
    fn from(item: &CommonPlaylistItem) -> Self {
        Self {
            virtual_id: item.virtual_id.get(),
            provider_id: Arc::clone(&item.provider_id),
            name: Arc::clone(&item.name),
            title: Arc::clone(&item.title),
            group: Arc::clone(&item.group),
            logo: pick_logo(&item.logo, &item.logo_small, item.additional_properties.as_ref()),
            url: Arc::clone(&item.url),
            item_type: item.item_type,
            xtream_cluster: item.xtream_cluster.unwrap_or_default(),
            category_id: item.category_id.unwrap_or(0),
            rating: get_rating(item.additional_properties.as_ref()),
            input_name: Arc::clone(&item.input_name),
            epg_channel_id: item.epg_channel_id.clone(),
        }
    }
}

impl From<XtreamPlaylistItem> for UiPlaylistItem {
    fn from(item: XtreamPlaylistItem) -> Self {
        Self {
            virtual_id: item.virtual_id.get(),
            provider_id: item.provider_id.to_string().into(),
            name: Arc::clone(&item.name),
            title: Arc::clone(&item.title),
            group: Arc::clone(&item.group),
            logo: pick_logo(&item.logo, &item.logo_small, item.additional_properties.as_ref()),
            url: Arc::clone(&item.url),
            item_type: item.item_type,
            xtream_cluster: item.xtream_cluster,
            category_id: item.category_id,
            rating: get_rating(item.additional_properties.as_ref()),
            input_name: Arc::clone(&item.input_name),
            epg_channel_id: item.epg_channel_id,
        }
    }
}

impl From<M3uPlaylistItem> for UiPlaylistItem {
    fn from(item: M3uPlaylistItem) -> Self {
        Self {
            virtual_id: item.virtual_id.get(),
            provider_id: Arc::clone(&item.provider_id),
            name: Arc::clone(&item.name),
            title: Arc::clone(&item.title),
            group: Arc::clone(&item.group),
            logo: pick_logo(&item.logo, &item.logo_small, None),
            url: Arc::clone(&item.url),
            item_type: item.item_type,
            xtream_cluster: item.item_type.cluster(),
            category_id: 0,
            rating: 0.0,
            input_name: Arc::clone(&item.input_name),
            epg_channel_id: item.epg_channel_id,
        }
    }
}

impl From<&PlaylistItem> for UiPlaylistItem {
    fn from(item: &PlaylistItem) -> Self {
        let header = &item.header;
        Self {
            virtual_id: header.virtual_id.get(),
            provider_id: Arc::clone(&header.id),
            name: Arc::clone(&header.name),
            title: Arc::clone(&header.title),
            group: Arc::clone(&header.group),
            logo: pick_logo(&header.logo, &header.logo_small, header.additional_properties.as_ref()),
            url: Arc::clone(&header.url),
            item_type: header.item_type,
            xtream_cluster: header.xtream_cluster,
            category_id: header.category_id,
            rating: get_rating(header.additional_properties.as_ref()),
            input_name: Arc::clone(&header.input_name),
            epg_channel_id: header.epg_channel_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UiPlaylistItem;
    use crate::model::{
        PlaylistItem, PlaylistItemHeader, PlaylistItemType, SeriesStreamDetailEpisodeProperties,
        SeriesStreamDetailProperties, SeriesStreamProperties, StreamProperties, VirtualId, XtreamCluster,
        XtreamPlaylistItem,
    };
    use crate::utils::Internable;

    fn legacy_episode_container() -> XtreamPlaylistItem {
        let properties = SeriesStreamProperties {
            details: Some(SeriesStreamDetailProperties {
                year: None,
                seasons: None,
                episodes: Some(vec![SeriesStreamDetailEpisodeProperties {
                    id: 18478,
                    title: "56 Dias S01E01".intern(),
                    direct_source: "http://example.test/series/user/pass/997117.mp4".intern(),
                    season: 1,
                    episode_num: 1,
                    ..Default::default()
                }]),
            }),
            ..Default::default()
        };
        XtreamPlaylistItem::from(&PlaylistItem {
            header: PlaylistItemHeader {
                virtual_id: VirtualId::new(18477),
                title: "56 Dias S01E01".intern(),
                item_type: PlaylistItemType::SeriesInfo,
                xtream_cluster: XtreamCluster::Series,
                additional_properties: Some(StreamProperties::Series(Box::new(properties))),
                ..Default::default()
            },
        })
    }

    #[test]
    fn target_m3u_cached_episode_uses_playable_id_and_original_url() {
        let item = UiPlaylistItem::from_target_item(legacy_episode_container(), true);
        assert_eq!(item.virtual_id, 18478);
        assert_eq!(item.item_type, PlaylistItemType::Series);
        assert_eq!(item.url.as_ref(), "http://example.test/series/user/pass/997117.mp4");
        assert_eq!(item.title.as_ref(), "56 Dias S01E01");
    }

    #[test]
    fn target_xtream_containers_are_not_flattened() {
        let item = UiPlaylistItem::from_target_item(legacy_episode_container(), false);
        assert_eq!(item.virtual_id, 18477);
        assert_eq!(item.item_type, PlaylistItemType::SeriesInfo);
        assert!(item.url.is_empty());
    }

    #[test]
    fn target_m3u_real_single_episode_show_keeps_container_identity() {
        let mut container = legacy_episode_container();
        container.title = "56 Dias".intern();
        let item = UiPlaylistItem::from_target_item(container, true);
        assert_eq!(item.virtual_id, 18477);
        assert_eq!(item.item_type, PlaylistItemType::SeriesInfo);
        assert!(item.url.is_empty());
    }

    #[test]
    fn target_m3u_container_with_missing_source_keeps_container_identity() {
        let mut container = legacy_episode_container();
        if let Some(StreamProperties::Series(series)) = container.additional_properties.as_mut() {
            series.details.as_mut().unwrap().episodes.as_mut().unwrap()[0].direct_source = "".intern();
        }
        let item = UiPlaylistItem::from_target_item(container, true);
        assert_eq!(item.virtual_id, 18477);
        assert_eq!(item.item_type, PlaylistItemType::SeriesInfo);
    }

    #[test]
    fn target_m3u_multi_episode_container_keeps_container_identity() {
        let mut container = legacy_episode_container();
        if let Some(StreamProperties::Series(series)) = container.additional_properties.as_mut() {
            let episodes = series.details.as_mut().unwrap().episodes.as_mut().unwrap();
            let mut second = episodes[0].clone();
            second.id = 18480;
            second.title = "56 Dias S01E02".intern();
            episodes.push(second);
        }
        let item = UiPlaylistItem::from_target_item(container, true);
        assert_eq!(item.virtual_id, 18477);
        assert_eq!(item.item_type, PlaylistItemType::SeriesInfo);
    }
}
