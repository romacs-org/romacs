//! GNU xml.c. VM2 links no libxml2, so it reports the parser absent, as a
//! GNU built without HAVE_LIBXML2 does; callers such as dbus.el then parse
//! with xml.el.
use super::*;

defsubr! { name = "libxml-available-p", gnu = "xml.c:321", effect = Pure,
fn libxml_available_p(cx) {
    let _ = cx;
    Ok(Value::NIL)
}}
