//! The VM's window system (romacs_vm::DisplayHost, DIVERGENCES.md
//! VM2-WIN5 and VM2-FONT1): frames on the GUI's display are GNU pgtk's,
//! and the fonts Lisp lists, opens and asks about are the ones painting
//! draws with. A character's font is resolved as layout resolves it: the
//! face's family at its pixel size, with no fontset of its own.
use romacs_font::{FontRequest, FontResolver, FontsetPolicy, entity};
use romacs_vm::{FontEntity, FontQuery, OpenedFont};
use std::collections::HashMap;

/// Romacs's icon font (assets/fonts/RomacsIcons.ttf, DIVERGENCES.md
/// VM2-MODE-LINE): under `ROMACS_RUNTIME_ROOT' (an installed
/// share/romacs), else in the checkout this binary was built from.
pub fn icon_font() -> std::path::PathBuf {
    std::env::var_os("ROMACS_RUNTIME_ROOT").map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .join("assets/fonts/RomacsIcons.ttf")
}

/// Make the icon font a Fontconfig application font, so a character only
/// it has resolves to it as any font's fallback does. Run before a
/// resolver is built, so none has cached its codepoints as missing. False
/// when the file is absent or Fontconfig refused it.
pub fn register_icon_font() -> bool {
    let font = icon_font();
    font.is_file() && romacs_font::register_font_file(&font)
}

pub struct Display {
    resolver: FontResolver,
    policy: FontsetPolicy,
    /// Opened faces by file, index and pixel size, and characters' fonts by
    /// family, pixel size and character: `frame-parameters' asks for the
    /// frame's font each time.
    opened: HashMap<(String, u32, i64), Option<OpenedFont>>,
    chars: HashMap<(String, u32, char), Option<(OpenedFont, u32)>>,
    /// Where `alpha' setters go, with the frame's display id: the window's
    /// opacity state.
    alpha_hook: Option<Box<dyn FnMut(u64, [f32; 2], f32)>>,
}

impl Display {
    pub fn new() -> Result<Self, String> {
        Ok(Self { resolver: FontResolver::new().map_err(|e| e.to_string())?, policy: FontsetPolicy::new(),
                  opened: HashMap::new(), chars: HashMap::new(), alpha_hook: None })
    }
    /// This display, sending each `alpha' setter to HOOK.
    pub fn with_alpha_hook(self, hook: impl FnMut(u64, [f32; 2], f32) + 'static) -> Self {
        Self { alpha_hook: Some(Box::new(hook)), ..self }
    }
    fn open_uncached(entity: &FontEntity, pixel_size: i64) -> Option<OpenedFont> {
        let metrics = romacs_font::probe::probe_font_px_metrics(&entity.file, entity.index, u32::try_from(pixel_size).ok()?.max(1), None)?;
        Some(OpenedFont {
            entity: entity.clone(),
            pixel_size: i64::from(metrics.pixel_size),
            ascent: i64::from(metrics.ascent),
            descent: i64::from(metrics.descent),
            max_width: i64::from(metrics.max_width),
            space_width: i64::from(metrics.space_width),
            average_width: i64::from(metrics.average_width),
            otf: romacs_font::probe::otf_capability(&entity.file, entity.index & 0xffff)
                .map_or_else(Default::default, |otf| (otf_table(otf.gsub), otf_table(otf.gpos))),
        })
    }
    fn char_font_uncached(&mut self, family: &str, pixel_size: f32, ch: char) -> Option<(OpenedFont, u32)> {
        let resolved = self.resolver.resolve_char(&FontRequest::new(family, pixel_size), ch, &self.policy).ok()?;
        let file = resolved.font.identity.file_path.clone()?;
        let index = resolved.font.identity.freetype_selector()?;
        let entity = entity::find(&resolved.font.family, &file, index).map(font_entity)?;
        let opened = romacs_vm::DisplayHost::open_font(self, &entity, resolved.font.pixel_size.round() as i64)?;
        Some((opened, resolved.glyph.glyph_id.get()))
    }
}

fn font_entity(entity: entity::Entity) -> FontEntity {
    FontEntity {
        foundry: entity.foundry,
        family: entity.family,
        weight: entity.weight,
        slant: entity.slant,
        width: entity.width,
        spacing: entity.spacing,
        registry: "iso10646-1".to_owned(),
        file: entity.file,
        index: entity.index,
    }
}

fn otf_table(scripts: Vec<romacs_font::probe::OtfScript>) -> romacs_vm::OtfTable {
    scripts.into_iter().map(|script| (script.tag, script.lang_syses.into_iter().map(|lang| (lang.tag, lang.features)).collect())).collect()
}

/// Whether ENTITY has each property QUERY gives (the pattern ftfont.c
/// hands FcFontList). Every face Romacs lists is a Unicode face, so any
/// registry matches.
fn matches(query: &FontQuery, entity: &FontEntity) -> bool {
    query.foundry.as_ref().is_none_or(|foundry| entity.foundry.as_ref().is_some_and(|f| f.eq_ignore_ascii_case(foundry)))
        && query.weight.is_none_or(|weight| weight == entity.weight)
        && query.slant.is_none_or(|slant| slant == entity.slant)
        && query.width.is_none_or(|width| width == entity.width)
        && query.spacing.is_none_or(|spacing| Some(spacing) == entity.spacing)
}

impl romacs_vm::DisplayHost for Display {
    fn window_system(&self) -> &str {
        "pgtk"
    }
    fn display_name(&self) -> String {
        std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".to_owned())
    }
    fn list_fonts(&mut self, query: &FontQuery) -> Vec<FontEntity> {
        entity::list(query.family.as_deref()).into_iter().map(font_entity).filter(|entity| matches(query, entity)).collect()
    }
    fn font_families(&mut self) -> Vec<String> {
        entity::families()
    }
    fn open_font(&mut self, entity: &FontEntity, pixel_size: i64) -> Option<OpenedFont> {
        self.opened.entry((entity.file.clone(), entity.index, pixel_size))
            .or_insert_with(|| Self::open_uncached(entity, pixel_size))
            .clone()
            .map(|opened| OpenedFont { entity: entity.clone(), ..opened })
    }
    fn char_font(&mut self, family: &str, pixel_size: f32, ch: char) -> Option<(OpenedFont, u32)> {
        let key = (family.to_owned(), pixel_size.to_bits(), ch);
        if let Some(found) = self.chars.get(&key) { return found.clone(); }
        let found = self.char_font_uncached(family, pixel_size, ch);
        self.chars.insert(key, found.clone());
        found
    }
    fn set_frame_alpha(&mut self, frame: u64, alpha: [f32; 2], limit: f32) {
        if let Some(hook) = &mut self.alpha_hook { hook(frame, alpha, limit); }
    }
}

#[cfg(test)]
mod tests {
    //! The GUI's frame on its display against pinned GNU pgtk's, `emacs -Q
    //! --font "PlemolJP Console NF-14"' (19 pixels) with that one font file
    //! added to the gate's Fontconfig: a face no host lists, which the test
    //! registers, so its answers are the fixture's own.
    use super::*;
    use romacs_vm::Vm;

    const FONT: &str = "-TWR-PlemolJP Console NF-regular-normal-normal-*-19-*-*-*-d-0-iso10646-1";

    fn displayed() -> Vm {
        assert!(romacs_font::register_font_file(romacs_test_fonts::plemol_jp_console_nf_regular()));
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 24, 10).unwrap();
        vm.set_display_host(Some(Box::new(Display::new().unwrap())));
        vm.eval_source("(progn (internal-set-lisp-face-attribute 'default :family \"PlemolJP Console NF\" (selected-frame))
                               (internal-set-lisp-face-attribute 'default :height 140 (selected-frame)))").unwrap();
        vm
    }
    fn printed(vm: &mut Vm, source: &str) -> String {
        vm.eval_source(source).unwrap_or_else(|e| panic!("{source}: {e}")).display().unwrap()
    }

    /// A character in the icons' range resolves to Romacs's icon font, for
    /// a face of any family: it is the only font with it.
    #[test]
    fn an_icon_resolves_to_the_icon_font() {
        assert!(register_icon_font());
        let mut resolver = FontResolver::new().unwrap();
        for family in ["Liberation Mono", "PlemolJP Console NF"] {
            let resolved = resolver.resolve_char(&FontRequest::new(family, 15.0), '\u{FE001}', &FontsetPolicy::new()).unwrap();
            let file = resolved.font.identity.file_path.clone().unwrap();
            assert_eq!(std::path::Path::new(&file).canonicalize().unwrap(), icon_font().canonicalize().unwrap(), "{family}");
            assert_ne!(resolved.glyph.glyph_id.get(), 0, "{family}: the glyph, not .notdef");
        }
    }

    #[test]
    fn a_frame_on_the_display_is_a_pgtk_frame() {
        let mut vm = displayed();
        // Until the session opens the display: the batch chain's initial
        // terminal.
        assert_eq!(printed(&mut vm, "(list (framep (selected-frame)) (window-system) (frame-initial-p) (frame-parameter nil 'font))"),
                   r#"(t nil t "tty")"#);
        assert!(vm.open_display());
        assert_eq!(printed(&mut vm, "(list (framep (selected-frame)) (frame-live-p (selected-frame)) (window-system)
                                          (terminal-live-p nil) (frame-initial-p) (frame-parameter nil 'window-system)
                                          (frame-parameter nil 'font-backend) (frame-parameter nil 'font))"),
                   format!(r#"(pgtk pgtk pgtk pgtk nil pgtk (ftcrhb) "{FONT}")"#));
    }

    /// `alpha' and `alpha-background' against pinned GNU's X11 frame, whose
    /// setters are pgtk's (frame.c gui_set_alpha, gui_set_alpha_background):
    /// each value is stored, then validated; painting gets the background
    /// opacity the last valid setter applied, and every valid `alpha' setter
    /// reaches the window, as GNU's set_frame_alpha_hook. Without a window
    /// system they are only stored.
    #[test]
    fn opacity_parameters_validate_as_gnu_and_reach_the_window() {
        let setters = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
        let seen = setters.clone();
        let mut vm = displayed();
        vm.set_display_host(Some(Box::new(Display::new().unwrap().with_alpha_hook(move |frame, alpha, limit| seen.borrow_mut().push((frame, alpha, limit))))));
        let set = |vm: &mut Vm, alist: &str| printed(vm, &format!(
            "(list (condition-case e (modify-frame-parameters nil '{alist}) (error e))
                   (frame-parameter nil 'alpha-background) (frame-parameter nil 'alpha))"));
        let background = |vm: &mut Vm| vm.frame_snapshot(1200, 800).unwrap().background_alpha;
        assert_eq!(set(&mut vm, "((alpha-background . 101) (alpha . 300))"), "(nil 101 300)");
        assert_eq!((background(&mut vm), setters.borrow().len()), (1.0, 0));
        assert!(vm.open_display());
        assert_eq!(printed(&mut vm, "(list frame-alpha-lower-limit (assq 'alpha (frame-parameters))
                                          (assq 'borders-respect-alpha-background (frame-parameters)))"),
                   "(20 (alpha . 300) (borders-respect-alpha-background))");
        assert_eq!(set(&mut vm, "((alpha-background . 80))"), "(nil 80 300)");
        assert_eq!(background(&mut vm), 0.8);
        assert_eq!(set(&mut vm, "((alpha-background . 101))"), "((args-out-of-range 0 100) 101 300)");
        assert_eq!(set(&mut vm, "((alpha-background . 1.5))"), "((args-out-of-range 0.0 1.0) 1.5 300)");
        assert_eq!(set(&mut vm, "((alpha-background . \"80\"))"), r#"((wrong-type-argument numberp "80") "80" 300)"#);
        assert_eq!(background(&mut vm), 0.8, "a refused value leaves the applied one");
        // Both setters of one call reach the window, the later last.
        assert_eq!(set(&mut vm, "((alpha) (alpha . (50 . 70)))"), "(nil \"80\" nil)");
        assert_eq!(*setters.borrow(), [(1, [0.5, 0.7], 0.2), (1, [-1.0, -1.0], 0.2)]);
        // Stored in reverse order, each before its handler: `alpha' signals
        // and `alpha-background' is never reached.
        assert_eq!(set(&mut vm, "((alpha-background . 40) (alpha . (a . 50)))"),
                   "((wrong-type-argument numberp a) \"80\" (a . 50))");
        assert_eq!(setters.borrow().len(), 2);
        assert_eq!(set(&mut vm, "((alpha-background))"), "(nil nil (a . 50))");
        assert_eq!(background(&mut vm), 1.0);
    }

    #[test]
    fn fonts_open_and_answer_as_gnu_pgtk_s() {
        let mut vm = displayed();
        vm.open_display();
        // The frame's font, its file by base name.
        assert_eq!(printed(&mut vm, "(let ((info (font-info (frame-parameter nil 'font))))
            (aset info 12 (file-name-nondirectory (aref info 12))) (aset info 13 (car (aref info 13))) info)"),
            format!(r#"["{FONT}" "PlemolJP Console NF:pixelsize=19:foundry=TWR:weight=regular:slant=normal:width=normal:spacing=90:scalable=true" 19 24 0 0 0 10 19 5 10 10 "PlemolJPConsoleNF-Regular-v3.1.0.ttf" opentype]"#));
        assert_eq!(printed(&mut vm, "(let ((info (font-info \"PlemolJP Console NF:pixelsize=30\")) (i 12) items)
            (while (> i 2) (setq i (1- i) items (cons (aref info i) items))) items)"),
                   "(30 36 0 0 0 16 29 7 16 16)");
        assert_eq!(printed(&mut vm, "(let ((otf (font-get (car (internal-char-font nil ?a)) :otf))) (list (length (car otf)) (car (car (car otf))) (car (car (cdr otf)))))"),
                   "(6 DFLT DFLT)");
        // λ and a: the frame's font and GNU's glyph codes.
        assert_eq!(printed(&mut vm, "(let ((lambda (internal-char-font nil 955)) (a (internal-char-font nil 97)) (font (frame-parameter nil 'font)))
            (list (list (fontp (car lambda) 'font-object) (equal (font-xlfd-name (car lambda)) font) (cdr lambda))
                  (list (fontp (car a) 'font-object) (equal (font-xlfd-name (car a)) font) (cdr a))))"),
                   "((t t 439) (t t 68))");
        assert_eq!(printed(&mut vm, "(let ((e (find-font (font-spec :family \"PlemolJP Console NF\"))))
            (list (fontp e 'font-entity) (font-get e :family) (font-get e :weight) (font-get e :slant) (font-get e :foundry)
                  (font-get e :spacing) (font-get e :registry) (font-get e :size)
                  (find-font (font-spec :family \"PlemolJP Console NF\" :weight 'bold))))"),
                   "(t PlemolJP Console NF regular normal TWR 90 iso10646-1 0 nil)");
        assert_eq!(printed(&mut vm, "(list (find-font (font-spec :family \"NoSuchFontXYZ\")) (font-info \"NoSuchFontXYZ-12\")
            (and (member \"PlemolJP Console NF\" (font-family-list)) t))"), "(nil nil t)");
        assert_eq!(printed(&mut vm, "(equal (font-xlfd-name (font-at 1 nil \"a\\u03bb\")) (frame-parameter nil 'font))"), "t");
    }

    /// xfaces.c Fface_font: on the display every face names the font
    /// painting draws it with, from the face's family and pixel size, and
    /// that font opens, so `window-font-width' (font.c `font-info' of
    /// `face-font'), by which hermes sizes its tables in `fixed-pitch',
    /// measures a face other than `default' instead of signalling. A face
    /// the display theme does not hold paints, and so names, the default
    /// face's font.
    #[test]
    fn every_face_names_the_font_it_draws() {
        let mut vm = displayed();
        vm.open_display();
        assert_eq!(printed(&mut vm, "(progn (internal-make-lisp-face 'rt-big) (internal-make-lisp-face 'rt-big (selected-frame))
            (internal-set-lisp-face-attribute 'rt-big :height 280 (selected-frame))
            (internal-set-lisp-face-attribute 'rt-big :weight 'bold t)
            (let ((font (face-font 'rt-big)))
              (list font (aref (font-info font) 2) (aref (font-info font) 11)
                    (equal (face-font \"default\" nil ?a) (frame-parameter nil 'font)) (face-font 'rt-big t)
                    (progn (internal-make-lisp-face 'rt-empty) (equal (face-font 'rt-empty) (frame-parameter nil 'font)))
                    (condition-case e (face-font 'rt-big nil 'x) (error e)) (condition-case e (face-font 'rt-nosuch) (error e)))))"),
                   r#"("-TWR-PlemolJP Console NF-regular-normal-normal-*-37-*-*-*-d-0-iso10646-1" 37 20 t (bold) t (wrong-type-argument characterp x) (error "Invalid face" rt-nosuch))"#);
    }

    /// The init's `initial-frame-alist' and `default-frame-alist' reach the
    /// display's frame once, before `window-setup-hook', on both of
    /// startup's paths, so the hook's own setting stays: pinned GNU X11
    /// with this init answers (80 90 50) with and without the screen.
    #[test]
    fn startup_applies_the_inits_frame_alists_once() {
        for screen in ["nil", "t"] {
            let answer = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
                let mut vm = displayed();
                let home = std::env::temp_dir().join(format!("romacs-fonts-alists-{}-{screen}", std::process::id()));
                std::fs::create_dir_all(&home).unwrap();
                vm.eval_source(&format!("(setq process-environment (append (list \"HOME={0}\" \"XDG_CONFIG_HOME={0}/config\") process-environment))",
                                        home.display())).unwrap();
                let started = crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::Loadup, None).unwrap();
                printed(&mut vm, &format!("(progn (setq inhibit-startup-screen {screen})
                    (setf (alist-get 'alpha-background default-frame-alist) 80)
                    (setq initial-frame-alist '((alpha . 90)))
                    (defvar probe-seen nil)
                    (add-hook 'window-setup-hook
                      (lambda ()
                        (setq probe-seen (list (frame-parameter nil 'alpha-background) (frame-parameter nil 'alpha)))
                        (set-frame-parameter nil 'alpha-background 50)))
                    t)"));
                crate::startup::finish_loadup(&mut vm, &started).unwrap();
                let answer = printed(&mut vm, "(append probe-seen (list (frame-parameter nil 'alpha-background)))");
                let background = vm.frame_snapshot(1200, 800).unwrap().background_alpha;
                let _ = std::fs::remove_dir_all(&home);
                (answer, background)
            }).unwrap().join().unwrap();
            assert_eq!(answer, ("(80 90 50)".to_owned(), 0.5), "inhibit-startup-screen {screen}");
        }
    }

    /// `--loadup` on the GUI's display: the session puts the frame on it, a
    /// `pgtk' frame as pinned GNU pgtk's, and a user's init, which turns
    /// prettify-symbols on in Lisp modes, lets startup finish: prettify asks
    /// `char-displayable-on-frame-p' about λ, which needs `fontp', when
    /// *scratch* takes `initial-major-mode', and then `emacs-startup-hook'
    /// runs; and the mode line's `%z%Z' show a buffer's coding, as on a
    /// window-system frame. Pinned GNU pgtk with this init: (pgtk pgtk pgtk
    /// t (lisp-interaction-mode t) (t t t) "UU:"). Then pgtk-win.el's
    /// selection methods keep this session's CLIPBOARD until another
    /// program copies, and frame.el and browse-url dispatch to the pgtk
    /// primitives: GNU pgtk on Wayland reports an undecorated frame at
    /// (0 . 0) and "GdkWaylandDisplay".
    #[test]
    fn loadup_on_the_display_finishes_startup_with_the_prettify_init() {
        let answer = std::thread::Builder::new().stack_size(64 << 20).spawn(|| {
            let mut vm = displayed();
            let home = std::env::temp_dir().join(format!("romacs-fonts-prettify-{}", std::process::id()));
            std::fs::create_dir_all(&home).unwrap();
            vm.eval_source(&format!("(setq process-environment (append (list \"HOME={0}\" \"XDG_CONFIG_HOME={0}/config\") process-environment))",
                                    home.display())).unwrap();
            let started = crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::Loadup, None).unwrap();
            printed(&mut vm, "(progn (add-hook 'emacs-lisp-mode-hook #'prettify-symbols-mode)
                (add-hook 'emacs-startup-hook (lambda () (setq prettify-startup-ran t))) t)");
            crate::startup::finish_loadup(&mut vm, &started).unwrap();
            let answer = printed(&mut vm, "(list window-system initial-window-system (framep (selected-frame)) (bound-and-true-p prettify-startup-ran)
                (with-current-buffer \"*scratch*\" (list major-mode prettify-symbols-mode))
                (list (fontp (char-displayable-p 955)) (char-displayable-on-frame-p 955) (char-displayable-p ?a))
                (with-temp-buffer (setq buffer-file-coding-system 'utf-8-unix) (format-mode-line \"%z%Z\")))");
            // pgtk-win.el's selection methods: this session's CLIPBOARD until
            // another program copies; frame.el's and browse-url's pgtk
            // dispatch on an undecorated Wayland frame.
            let pgtk = printed(&mut vm, "(let ((desktop nil))
                (cl-letf (((symbol-function 'romacs--clipboard-set) (lambda (text) (setq desktop text)))
                          ((symbol-function 'romacs--clipboard-text) (lambda () desktop)))
                  (gui-set-selection 'CLIPBOARD \"A\")
                  (let ((mine (list desktop (gui-get-selection 'CLIPBOARD) (gui-backend-selection-owner-p 'CLIPBOARD))))
                    (setq desktop \"B\")
                    (list mine (gui-get-selection 'CLIPBOARD) (gui-backend-selection-owner-p 'CLIPBOARD)
                          ;; Copied again, then the desktop holds no text.
                          (progn (gui-set-selection 'CLIPBOARD \"C\") (setq desktop nil)
                                 (list (gui-get-selection 'CLIPBOARD) (gui-backend-selection-owner-p 'CLIPBOARD)))
                          (frame-edges nil 'outer-edges) (cdr (assq 'outer-size (frame-geometry)))
                          (mouse-absolute-pixel-position) (let ((process-environment (cons \"WAYLAND_DISPLAY=wayland-0\" process-environment))) (pgtk-backend-display-class))))))");
            let answer = format!("{answer} {pgtk}");
            let _ = std::fs::remove_dir_all(&home);
            answer
        }).unwrap().join().unwrap();
        assert_eq!(answer, r#"(pgtk pgtk pgtk t (lisp-interaction-mode t) (t t t) "UU:") (("A" "A" t) "B" nil (nil nil) (0 0 1200 800) (1200 . 800) (0 . 0) "GdkWaylandDisplay")"#);
    }
}
