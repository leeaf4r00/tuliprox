use crate::{
    model::{PlaylistItemType, SearchRequest, StreamProperties, UiPlaylistItem, VirtualId, XtreamCluster},
    utils::{arc_str_option_serde, arc_str_serde, format_float_localized},
};
use serde::{Deserialize, Serialize};
use std::{rc::Rc, sync::Arc};

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct PlaylistRequestXtream {
    pub username: String,
    pub password: String,
    pub url: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct PlaylistRequestM3u {
    pub url: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum PlaylistRequest {
    Target(u16),
    Input(String),
    CustomXtream(PlaylistRequestXtream),
    CustomM3u(PlaylistRequestM3u),
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum PlaylistUrlResolveRequest {
    Webplayer { target_id: u16, virtual_id: u32, cluster: XtreamCluster },
    Provider { playlist_request: PlaylistRequest, url: String },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CommonPlaylistItem {
    pub virtual_id: VirtualId,
    #[serde(with = "arc_str_serde")]
    pub provider_id: Arc<str>,
    #[serde(with = "arc_str_serde")]
    pub name: Arc<str>,
    pub chno: u32,
    #[serde(with = "arc_str_serde")]
    pub logo: Arc<str>,
    #[serde(with = "arc_str_serde")]
    pub logo_small: Arc<str>,
    #[serde(with = "arc_str_serde")]
    pub group: Arc<str>,
    #[serde(with = "arc_str_serde")]
    pub title: Arc<str>,
    #[serde(with = "arc_str_serde")]
    pub parent_code: Arc<str>,
    #[serde(with = "arc_str_serde")]
    pub audio_track: Arc<str>,
    #[serde(with = "arc_str_serde")]
    pub time_shift: Arc<str>,
    #[serde(with = "arc_str_serde")]
    pub rec: Arc<str>,
    #[serde(with = "arc_str_serde")]
    pub url: Arc<str>,
    #[serde(with = "arc_str_serde")]
    pub input_name: Arc<str>,
    pub item_type: PlaylistItemType,
    #[serde(default, with = "arc_str_option_serde")]
    pub epg_channel_id: Option<Arc<str>>,
    #[serde(default)]
    pub xtream_cluster: Option<XtreamCluster>,
    #[serde(default)]
    pub additional_properties: Option<StreamProperties>,
    #[serde(default)]
    pub category_id: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct UiPlaylistGroup {
    pub id: u32,
    #[serde(with = "arc_str_serde")]
    pub title: Arc<str>,
    pub channels: Vec<Rc<UiPlaylistItem>>,
    pub xtream_cluster: XtreamCluster,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct UiPlaylistCategories {
    #[serde(default)]
    pub live: Option<Vec<Rc<UiPlaylistGroup>>>,
    #[serde(default)]
    pub vod: Option<Vec<Rc<UiPlaylistGroup>>>,
    #[serde(default)]
    pub series: Option<Vec<Rc<UiPlaylistGroup>>>,
}

pub const SEARCH_FIELD_GROUP: &str = "group";
pub const SEARCH_FIELD_TITLE: &str = "title";
pub const SEARCH_FIELD_NAME: &str = "name";
pub const SEARCH_FIELD_RATING: &str = "rating";
pub const SEARCH_FIELD_URL: &str = "url";

#[derive(Debug, Clone, Copy)]
struct SearchFieldMask {
    group: bool,
    title: bool,
    name: bool,
    rating: bool,
    url: bool,
}

impl SearchFieldMask {
    // Legacy scope used when no fields are selected: group title + channel title/name.
    const DEFAULT: Self = Self { group: true, title: true, name: true, rating: false, url: false };

    fn from_search_fields(fields: Option<&Vec<String>>) -> Self {
        let Some(fields) = fields.filter(|f| !f.is_empty()) else {
            return Self::DEFAULT;
        };
        let mut mask = Self { group: false, title: false, name: false, rating: false, url: false };
        for field in fields {
            match field.as_str() {
                SEARCH_FIELD_GROUP => mask.group = true,
                SEARCH_FIELD_TITLE => mask.title = true,
                SEARCH_FIELD_NAME => mask.name = true,
                SEARCH_FIELD_RATING => mask.rating = true,
                SEARCH_FIELD_URL => mask.url = true,
                _ => {}
            }
        }
        if mask.group || mask.title || mask.name || mask.rating || mask.url {
            mask
        } else {
            Self::DEFAULT
        }
    }
}

fn filter_channels(
    groups: Option<&Vec<Rc<UiPlaylistGroup>>>,
    mask: SearchFieldMask,
    matches: &dyn Fn(&str) -> bool,
) -> Option<Vec<Rc<UiPlaylistGroup>>> {
    groups.map(|gs| {
        gs.iter()
            .filter_map(|group| {
                if mask.group && matches(&group.title) {
                    return Some(Rc::clone(group));
                }

                let filtered_channels: Vec<Rc<UiPlaylistItem>> = group
                    .channels
                    .iter()
                    .filter(|c| {
                        (mask.title && matches(&c.title))
                            || (mask.name && matches(&c.name))
                            || (mask.rating && rating_matches(c.rating, matches))
                            || (mask.url && matches(&c.url))
                    })
                    .cloned()
                    .collect();

                if filtered_channels.is_empty() {
                    None
                } else {
                    Some(Rc::new(UiPlaylistGroup {
                        id: group.id,
                        title: group.title.clone(),
                        channels: filtered_channels,
                        xtream_cluster: group.xtream_cluster,
                    }))
                }
            })
            .collect::<Vec<_>>()
    })
}

fn rating_matches(rating: f64, matches: &dyn Fn(&str) -> bool) -> bool {
    if !rating.is_finite() || rating <= 0.001 {
        return false;
    }

    let localized_rating = format_float_localized(rating, 1, false);
    matches(&localized_rating) || matches(&localized_rating.replace(',', "."))
}

fn build_result(
    live: Option<Vec<Rc<UiPlaylistGroup>>>,
    vod: Option<Vec<Rc<UiPlaylistGroup>>>,
    series: Option<Vec<Rc<UiPlaylistGroup>>>,
) -> Option<UiPlaylistCategories> {
    if live.is_none() && vod.is_none() && series.is_none() {
        None
    } else {
        Some(UiPlaylistCategories { live, vod, series })
    }
}

impl UiPlaylistCategories {
    pub fn filter(&self, search_req: &SearchRequest) -> Option<Self> {
        match search_req {
            SearchRequest::Clear => None,
            SearchRequest::Text(text, search_fields) => {
                let mask = SearchFieldMask::from_search_fields(search_fields.as_deref());
                let text_lc = text.to_lowercase();
                let matches = |value: &str| value.to_lowercase().contains(&text_lc);
                let live = filter_channels(self.live.as_ref(), mask, &matches);
                let video = filter_channels(self.vod.as_ref(), mask, &matches);
                let series = filter_channels(self.series.as_ref(), mask, &matches);
                build_result(live, video, series)
            }
            SearchRequest::Regexp(text, search_fields) => {
                if let Ok(regex) = crate::model::REGEX_CACHE.get_or_compile(text) {
                    let mask = SearchFieldMask::from_search_fields(search_fields.as_deref());
                    let matches = |value: &str| regex.is_match(value);
                    let live = filter_channels(self.live.as_ref(), mask, &matches);
                    let video = filter_channels(self.vod.as_ref(), mask, &matches);
                    let series = filter_channels(self.series.as_ref(), mask, &matches);
                    build_result(live, video, series)
                } else {
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PlaylistItemType, UiPlaylistGroup, XtreamCluster};
    use std::sync::Arc;

    fn rated_item(title: &str, rating: f64) -> Rc<UiPlaylistItem> {
        Rc::new(UiPlaylistItem {
            virtual_id: 1,
            provider_id: Arc::from("provider-id"),
            name: Arc::from(title),
            title: Arc::from(title),
            group: Arc::from("Movies"),
            logo: Arc::from(""),
            url: Arc::from("https://example.test/movie"),
            item_type: PlaylistItemType::Video,
            xtream_cluster: XtreamCluster::Video,
            category_id: 1,
            rating,
            input_name: Arc::from("test"),
            epg_channel_id: None,
        })
    }

    fn rated_movies() -> UiPlaylistCategories {
        UiPlaylistCategories {
            live: None,
            vod: Some(vec![Rc::new(UiPlaylistGroup {
                id: 1,
                title: Arc::from("Releases"),
                channels: vec![rated_item("Movie 8.4", 8.4), rated_item("Movie 5.9", 5.9), rated_item("Unrated", 0.0)],
                xtream_cluster: XtreamCluster::Video,
            })]),
            series: None,
        }
    }

    #[test]
    fn rating_search_matches_the_displayed_comma_or_dot_decimal() {
        let categories = rated_movies();

        for query in ["8,4", "8.4"] {
            let request = SearchRequest::Text(query.to_string(), Some(Rc::new(vec![SEARCH_FIELD_RATING.to_string()])));
            let filtered = categories.filter(&request).expect("rating search should return categories");
            let movies = filtered.vod.expect("VOD cluster should remain present");

            assert_eq!(movies.len(), 1);
            assert_eq!(movies[0].channels.len(), 1);
            assert_eq!(movies[0].channels[0].title.as_ref(), "Movie 8.4");
        }
    }

    #[test]
    fn rating_is_not_searched_by_default_or_when_unavailable() {
        let categories = rated_movies();
        let request = SearchRequest::Text("8,4".to_string(), None);
        let filtered = categories.filter(&request).expect("search should return categories");

        assert!(filtered.vod.as_ref().expect("VOD cluster should remain present").is_empty());
        assert!(!rating_matches(0.0, &|value| value.contains('0')));
        assert!(!rating_matches(f64::NAN, &|_| true));
    }
}
