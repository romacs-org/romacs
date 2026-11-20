//! A `FreeType` face's variation axes and named instances, and setting its
//! design coordinates: the two `FreeType` calls freetype-rs does not wrap,
//! made here once so the probe is safe code.

use freetype::Library;
use freetype::freetype_sys as ft;

/// One variation axis: its tag and its default design value (16.16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Axis {
    pub tag: ft::FT_ULong,
    pub default: ft::FT_Fixed,
}

/// A face's variation data, copied out of `FreeType`'s `FT_MM_Var`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Variations {
    pub axes: Vec<Axis>,
    /// Each named instance's design coordinates, one per axis.
    pub named_styles: Vec<Vec<ft::FT_Fixed>>,
}

/// FACE's variations, FACE opened by LIBRARY; None for a face without
/// them.
pub(super) fn of(library: &Library, face: &mut freetype::Face) -> Option<Variations> {
    let mut mm: *mut ft::FT_MM_Var = std::ptr::null_mut();
    // SAFETY: the face is live and mutably borrowed. On success FreeType
    // stores in `mm` a structure it allocated for the caller, whose `axis`
    // holds `num_axis` entries and `namedstyle` `num_namedstyles`, each
    // entry's `coords` one value per axis; every slice is copied (a null
    // pointer or zero count reads as empty) before `FT_Done_MM_Var` frees
    // the structure once. It frees through LIBRARY's memory, which is the
    // face's: every freetype-rs `Library` shares one malloc-backed
    // `FT_MemoryRec`, so even another library would free it correctly.
    // UNSAFE(freetype-rs 0.38 has no FT_Get_MM_Var, and ttf-parser's fvar
    // table has no named instances)
    unsafe {
        if ft::FT_Get_MM_Var(face.raw_mut(), &raw mut mm) != 0 || mm.is_null() {
            return None;
        }
        let var = &*mm;
        let axes: &[ft::FT_Var_Axis] = if var.num_axis == 0 || var.axis.is_null() {
            &[]
        } else {
            std::slice::from_raw_parts(var.axis, var.num_axis as usize)
        };
        let styles: &[ft::FT_Var_Named_Style] = if var.num_namedstyles == 0 || var.namedstyle.is_null() {
            &[]
        } else {
            std::slice::from_raw_parts(var.namedstyle, var.num_namedstyles as usize)
        };
        let variations = Variations {
            axes: axes.iter().map(|axis| Axis { tag: axis.tag, default: axis.def }).collect(),
            named_styles: styles
                .iter()
                .map(|style| {
                    if axes.is_empty() || style.coords.is_null() {
                        Vec::new()
                    } else {
                        std::slice::from_raw_parts(style.coords, axes.len()).to_vec()
                    }
                })
                .collect(),
        };
        ft::FT_Done_MM_Var(library.raw(), mm);
        Some(variations)
    }
}

/// Set FACE's design coordinates, one per axis.
pub(super) fn set_design_coordinates(face: &mut freetype::Face, coords: &[ft::FT_Fixed]) {
    // SAFETY: the face is live and mutably borrowed; `coords` holds the
    // count passed, and FreeType reads it only during the call, copying
    // what it keeps.
    // UNSAFE(freetype-rs 0.38 has no FT_Set_Var_Design_Coordinates)
    unsafe {
        ft::FT_Set_Var_Design_Coordinates(face.raw_mut(), coords.len() as ft::FT_UInt, coords.as_ptr());
    }
}
