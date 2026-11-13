use super::*;

/// Keep metadata separate: artist and title can themselves contain " - ".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Track {
    pub id: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub thumbnail: Option<String>,
}

impl Track {
    pub fn parse(value: &Value, token: Option<&str>) -> Option<Self> {
        let object = value.as_object()?;
        let clean =
            |value: &str| crate::output::safe_text(value, token).replace(['\n', '\r', '\t'], " ");
        let title = object
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Unknown track");
        let artist = object
            .get("author")
            .or_else(|| object.get("artist"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let album = object
            .get("album")
            .and_then(|album| {
                album
                    .as_str()
                    .or_else(|| album.get("name").and_then(Value::as_str))
            })
            .or_else(|| object.get("albumName").and_then(Value::as_str));
        Some(Self {
            id: object.get("id").and_then(Value::as_str).map(clean),
            title: clean(title),
            artist: clean(artist),
            album: album.filter(|value| !value.trim().is_empty()).map(clean),
            duration_ms: object
                .get("durationMs")
                .and_then(Value::as_u64)
                .filter(|value| *value > 0),
            thumbnail: object
                .get("thumbnail")
                .or_else(|| object.get("artworkUrl"))
                .and_then(Value::as_str)
                .filter(|value| value.len() <= 8192)
                .map(str::to_owned),
        })
    }

    pub fn same_recording(&self, other: &Self) -> bool {
        match (&self.id, &other.id) {
            (Some(a), Some(b)) => a == b,
            _ => self.title == other.title && self.artist == other.artist,
        }
    }

    pub fn label(&self) -> String {
        if self.artist.is_empty() {
            self.title.clone()
        } else {
            format!("{} — {}", self.title, self.artist)
        }
    }
}

// Handy for display-only fixtures. Network responses always use structured parsing.
#[cfg(test)]
impl From<String> for Track {
    fn from(value: String) -> Self {
        let (title, artist) = value.rsplit_once(" - ").unwrap_or((&value, ""));
        Self {
            title: title.into(),
            artist: artist.into(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
impl From<&str> for Track {
    fn from(value: &str) -> Self {
        value.to_string().into()
    }
}
