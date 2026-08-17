use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewPoint { pub x: i32, pub z: i32 }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerView {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub uuid: String,
    pub world: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")] pub y: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")] pub z: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")] pub yaw: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")] pub armor: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")] pub health: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayersView { pub players: Vec<PlayerView>, pub max: u32 }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldSummaryView {
    pub name: String,
    pub display_name: String,
    pub icon: String,
    #[serde(rename = "type")] pub environment: String,
    pub order: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiCoordinatesView { pub enabled: bool, pub html: String }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiLinkView { pub enabled: bool }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiSidebarView { pub pinned: String, pub player_list_label: String, pub world_list_label: String }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiView {
    pub title: String,
    pub coordinates: UiCoordinatesView,
    pub link: UiLinkView,
    pub sidebar: UiSidebarView,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsView { pub worlds: Vec<WorldSummaryView>, pub ui: UiView }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpawnView { pub x: i32, pub z: i32 }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NameplatesView {
    pub enabled: bool,
    pub show_heads: bool,
    pub heads_url: String,
    pub show_armor: bool,
    pub show_health: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerTrackerView {
    pub enabled: bool,
    pub update_interval: u32,
    pub label: String,
    pub show_controls: bool,
    pub default_hidden: bool,
    pub priority: i32,
    pub z_index: i32,
    pub nameplates: NameplatesView,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZoomView { pub max: i32, pub r#def: i32, pub extra: i32 }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldSettingsView {
    pub spawn: SpawnView,
    pub player_tracker: PlayerTrackerView,
    pub zoom: ZoomView,
    pub marker_update_interval: u32,
    pub tiles_update_interval: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarkerStyleView {
    #[serde(skip_serializing_if = "is_true")] pub stroke: bool,
    #[serde(skip_serializing_if = "is_default_stroke_color")] pub color: String,
    #[serde(skip_serializing_if = "is_default_weight")] pub weight: u32,
    #[serde(skip_serializing_if = "is_default_opacity")] pub opacity: f64,
    #[serde(skip_serializing_if = "is_true")] pub fill: bool,
    #[serde(rename = "fillColor", skip_serializing_if = "Option::is_none")] pub fill_color: Option<String>,
    #[serde(rename = "fillOpacity", skip_serializing_if = "is_default_fill_opacity")] pub fill_opacity: f64,
    #[serde(rename = "fillRule", skip_serializing_if = "is_default_fill_rule")] pub fill_rule: String,
}
fn is_true(v: &bool) -> bool { *v }
fn is_default_stroke_color(v: &String) -> bool { v == "#0000ff" }
fn is_default_weight(v: &u32) -> bool { *v == 3 }
fn is_default_opacity(v: &f64) -> bool { *v == 1.0 }
fn is_default_fill_opacity(v: &f64) -> bool { *v == 0.2 }
fn is_default_fill_rule(v: &String) -> bool { v == "evenodd" }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarkerTooltipView {
    #[serde(rename = "popup", skip_serializing_if = "Option::is_none")] pub click: Option<String>,
    #[serde(rename = "tooltip", skip_serializing_if = "Option::is_none")] pub hover: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MarkerGeometryView {
    #[serde(rename = "icon")] Icon { point: ViewPoint, size: ViewPoint, anchor: ViewPoint, tooltip_anchor: ViewPoint, icon: String },
    #[serde(rename = "circle")] Circle { center: ViewPoint, radius: f64 },
    #[serde(rename = "ellipse")] Ellipse { center: ViewPoint, #[serde(rename = "radiusX")] radius_x: f64, #[serde(rename = "radiusZ")] radius_z: f64 },
    #[serde(rename = "rectangle")] Rectangle { points: Vec<ViewPoint> },
    #[serde(rename = "polyline")] Polyline { points: PolylinePoints },
    #[serde(rename = "polygon")] Polygon { points: Vec<Vec<ViewPoint>> },
    #[serde(rename = "multipolygon")] MultiPolygon { points: Vec<Vec<Vec<ViewPoint>>> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PolylinePoints {
    Flat(Vec<ViewPoint>),
    Nested(Vec<Vec<ViewPoint>>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IconView {
    pub id: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IconsView {
    pub icons: Vec<IconView>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarkerView {
    #[serde(flatten)] pub style: MarkerStyleView,
    #[serde(flatten, skip_serializing_if = "Option::is_none")] pub tooltip: Option<MarkerTooltipView>,
    #[serde(flatten)] pub geometry: MarkerGeometryView,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarkerLayerView {
    pub id: String,
    pub name: String,
    pub control: bool,
    pub hide: bool,
    pub order: i32,
    pub z_index: i32,
    pub timestamp: u64,
    pub markers: Vec<MarkerView>,
}

pub fn serialize_json<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> { serde_json::to_vec(value) }
