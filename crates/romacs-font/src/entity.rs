//! GNU's font entities over the platform catalog: the installed faces
//! `list-fonts', `find-font' and `font-family-list' report, with their
//! styles as GNU's ftfont.c reads them from Fontconfig (font.c's numeric
//! weight, slant and width scales).

use crate::resolver::list_family;

/// One installed scalable face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    pub foundry: Option<String>,
    pub family: String,
    /// font.c weight_table's scale: 80 regular, 200 bold.
    pub weight: i64,
    /// font.c slant_table's scale: 100 normal, 200 italic, 210 oblique.
    pub slant: i64,
    /// font.c width_table's scale: 100 normal.
    pub width: i64,
    /// 0 proportional, 90 dual, 100 mono, 110 charcell.
    pub spacing: Option<i64>,
    pub file: String,
    /// Fontconfig's `FC_INDEX' (collection face and named instance).
    pub index: u32,
}

#[cfg(target_os = "linux")]
fn entity(face: crate::fontconfig_catalog::ListedFace) -> Entity {
    let (weight, slant, width) = face.fc_style;
    // ftfont.c ftfont_pattern_entity: weights from regular up to medium are
    // regular, and Fontconfig's slants sit 100 below GNU's.
    let weight = if (80..100).contains(&weight) { 80 } else { weight };
    Entity {
        foundry: face.foundry,
        family: face.family,
        weight: i64::from(weight),
        slant: i64::from(slant) + 100,
        width: i64::from(width),
        spacing: face.spacing.map(i64::from),
        file: face.file,
        index: face.selector,
    }
}

/// The installed faces of FAMILY, in Fontconfig's listing order; every
/// face when FAMILY is None. A generic family lists its Fontconfig match.
pub fn list(family: Option<&str>) -> Vec<Entity> {
    #[cfg(target_os = "linux")]
    {
        let faces = match family {
            Some(family) => list_family(family, None),
            None => crate::fontconfig_catalog::list_faces(None, None),
        };
        faces.into_iter().map(entity).collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (family, list_family);
        Vec::new()
    }
}

/// Every installed family name, as GNU's `font-family-list' reads them:
/// Fontconfig's distinct family sets, the first name of each, scalable or
/// not, in listing order.
pub fn families() -> Vec<String> {
    crate::resolver::catalog::list_families()
}

/// FAMILY's face in FILE at INDEX, as listed.
pub fn find(family: &str, file: &str, index: u32) -> Option<Entity> {
    list(Some(family)).into_iter().find(|entity| entity.file == file && entity.index == index)
}
