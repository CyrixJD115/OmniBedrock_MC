//! Mapping from website-style query slugs to CurseForge API parameters.
use super::consts::CLASSES;

/// Website class slug → CurseForge classId.
pub fn class_id(class: &str) -> Option<u64> {
    CLASSES
        .iter()
        .find(|(slug, _)| slug.eq_ignore_ascii_case(class))
        .map(|(_, id)| *id)
}

/// Website `sortBy` slug → undocumented-but-stable `ModsSearchSortField`.
/// Values 1/2/3/6 match the documented enum (Featured, Popularity,
/// LastUpdated, TotalDownloads); 10 sorts by creation date.
pub fn sort_field(sort: &str) -> Option<u64> {
    let normalized = sort.trim().to_ascii_lowercase().replace(['+', ' '], "-");
    match normalized.as_str() {
        "relevancy" | "featured" | "" => Some(1),
        "popularity" => Some(2),
        "latest-update" | "lastupdated" => Some(3),
        "creation-date" | "datecreated" => Some(10),
        "total-downloads" | "downloads" => Some(6),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_slugs_map_to_bedrock_ids() {
        assert_eq!(class_id("addons"), Some(4984));
        assert_eq!(class_id("Maps"), Some(6913));
        assert_eq!(class_id("texture-packs"), Some(6929));
        assert_eq!(class_id("scripts"), Some(6940));
        assert_eq!(class_id("skins"), Some(6925));
        assert_eq!(class_id("nonsense"), None);
    }

    #[test]
    fn sort_slugs_map_to_fields() {
        assert_eq!(sort_field("relevancy"), Some(1));
        assert_eq!(sort_field("popularity"), Some(2));
        assert_eq!(sort_field("latest+update"), Some(3));
        assert_eq!(sort_field("latest update"), Some(3));
        assert_eq!(sort_field("creation+date"), Some(10));
        assert_eq!(sort_field("total+downloads"), Some(6));
        assert_eq!(sort_field(""), Some(1));
        assert_eq!(sort_field("bogus"), None);
    }
}
