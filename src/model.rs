use enum_iterator::Sequence;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Sequence)]
pub enum TargetType {
    M3u,
    Strm,
}

impl std::fmt::Display for TargetType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            TargetType::M3u => write!(f, "M3u"),
            TargetType::Strm => write!(f, "Strm"),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Sequence)]
pub enum ProcessingOrder {
    Frm, Fmr, Rfm, Rmf, Mfr,  Mrf
}

impl std::fmt::Display for ProcessingOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            ProcessingOrder::Frm => write!(f, "filter, rename, map"),
            ProcessingOrder::Fmr => write!(f, "filter, map, rename"),
            ProcessingOrder::Rfm => write!(f, "rename, filter, map"),
            ProcessingOrder::Rmf => write!(f, "rename, map, filter"),
            ProcessingOrder::Mfr => write!(f, "map, filter, rename"),
            ProcessingOrder::Mrf => write!(f, "map, rename, filter"),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Sequence)]
pub enum ItemField {
    Group,
    Name,
    Title,
    Url,
}

impl std::fmt::Display for ItemField {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            ItemField::Group => write!(f, "Group"),
            ItemField::Name => write!(f, "Name"),
            ItemField::Title => write!(f, "Title"),
            ItemField::Url => write!(f, "Url"),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum FilterMode {
    Discard,
    Include,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SortOrder {
    Asc,
    Desc,
}