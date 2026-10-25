use super::*;

defsubr! { name = "text-quoting-style", gnu = "doc.c:652", effect = Reads,
fn text_quoting_style(cx) {
    let style = cx.text_quoting_style()?;
    Ok(cx.intern(style))
}}
