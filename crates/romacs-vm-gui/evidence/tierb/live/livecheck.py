"""Compositor-pixel checks shared by run.py and mutate_check.py.

The GUI's scene dump says what was painted, glyph by glyph, with each
glyph's cell position, background and the clip rectangle of its region (a
window's text area or its mode line). These checks prove the compositor
showed exactly that:

  * band background: a strip at the right end of each mode line has the
    dumped face colour;
  * cell ink, for EVERY region (all windows' text areas and mode lines):
    the region is cut into the frame's character cells; every cell holding a
    dumped non-space glyph must show glyph ink, and every other cell (dumped
    spaces and the empty cells right of or below the text) must show none.
    So missing text, missing digits, rows shifted by one line, a stale frame
    from another scroll position, mode-line text replaced by other marks and
    a blank window all fail. Two cells of slack per text area allow for the
    blinking cursor; none for mode lines.

  * mode-line flags ('-', '*', '%', ':'): each cell must have its
    character's geometry (a thin mid-height bar, an upper-half star, ...),
    which OCR reads unreliably and shape consistency cannot pin down;
  * every glyph (mode lines and text areas) against an INDEPENDENT
    reference: the GUI's font rasterised by ImageMagick, not by romacs; each
    cell must correlate with its own character (>= 0.90) and no
    distinguishable character of ASCII, Latin-1, Greek, punctuation, arrows,
    math or box drawing may match it as well, so a consistent substitution
    that keeps shapes self-consistent still fails (the cursor cell is exempt);
  * content, independently of the dump's own pixels: Tesseract reads every
    region and its rows must match the dumped rows in order (every number,
    nearly every character, modulo the font's known OCR confusions); each
    mode line must read with the dumped position (All/Top/Bottom/NN%, L<n>)
    and most of its words.

Masked only when present: if desktop notifications (orange
border, top-right corner) are drawn over the window in a screenshot, cells
inside NOTIFICATION_MASK are not judged for that screenshot, and the step
records that it was masked. Without notifications nothing is masked.
"""
import subprocess

# x >= 730 and y < 330, in window pixels: the notification popup stack.
NOTIFICATION_MASK = (730.0, 0.0, 1e9, 330.0)
INK = 0.008      # a '.' covers ~0.016 of a 10x25 cell after thresholding
BLANK = 0.004


def magick(*a):
    return subprocess.check_output(['magick', *map(str, a)], text=True).strip()


def mean_colour(png, x, y, w, h):
    return magick(png, '-crop', f'{int(w)}x{int(h)}+{int(x)}+{int(y)}', '+repage',
                  '-scale', '1x1!', '-format', '%[hex:p{0,0}]', 'info:').lower()[:6]


def close(a, b, tol=6):
    return all(abs(int(a[i:i + 2], 16) - int(b[i:i + 2], 16)) <= tol for i in (0, 2, 4))


def cell_ink(png, x, y, w, h, cw, ch, background):
    """Ink fraction per cell of a cw x ch grid over the region: pixels near
    the region's background colour count as empty, the rest are thresholded."""
    cols, rows = int(w // cw), int(h // ch)
    if cols <= 0 or rows <= 0:
        return [], 0, 0
    out = magick(png, '-crop', f'{int(cols * cw)}x{int(rows * ch)}+{int(x)}+{int(y)}', '+repage',
                 '-fuzz', '12%', '-fill', 'black', '-opaque', f'#{background}',
                 '-colorspace', 'gray', '-threshold', '25%',
                 '-filter', 'box', '-resize', f'{cols}x{rows}!', '-depth', '16', 'txt:-')
    grid = [[0.0] * cols for _ in range(rows)]
    for line in out.splitlines()[1:]:
        pos, rest = line.split(':', 1)
        c, r = map(int, pos.split(','))
        grid[r][c] = float(rest.split('(')[1].split(')')[0].split(',')[0]) / 65535.0
    return grid, cols, rows


def regions(glyphs):
    out = {}
    for g in glyphs:
        c = g['clip']
        out.setdefault((g['role'], c['x'], c['y'], c['width'], c['height']), []).append(g)
    return out


MASK_ON = False   # set per screenshot by check_step


def notifications_present(png):
    mx, my, _, mh = NOTIFICATION_MASK
    frac = float(magick(png, '-crop', f'10000x{int(mh)}+{int(mx)}+{int(my)}', '+repage',
                        '-fuzz', '15%', '-fill', 'white', '-opaque', '#d48a20', '-fill', 'black', '+opaque', 'white',
                        '-format', '%[fx:mean]', 'info:'))
    return frac > 0.01


def masked(x, y):
    mx, my, mw, mh = NOTIFICATION_MASK
    return MASK_ON and mx <= x < mx + mw and my <= y < my + mh


def check_cells(png, glyphs, dy=0, cw=10.0, ch=25.0):
    failures = []
    for (role, x, y, w, h), gs in regions(glyphs).items():
        background = (gs[0]['background'] if role == 'ModeLine' else '#000000').lstrip('#')
        grid, cols, rows = cell_ink(png, x, y - dy, w, h, cw, ch, background)
        want = {(int(round((g['y'] - y) / ch)), int(round((g['x'] - x) / cw))): bool(g['char'].strip()) for g in gs}
        missing, extra = [], []
        for r in range(rows):
            for c in range(cols):
                if masked(x + c * cw, y + r * ch):
                    continue
                if want.get((r, c)) and grid[r][c] < INK:
                    missing.append((r, c))
                elif not want.get((r, c)) and grid[r][c] >= BLANK:
                    extra.append((r, c))
        slack = 2 if role == 'Text' else 0
        if missing:
            failures.append(f'{role} region at ({x},{y}): {len(missing)} glyph cells show no ink, e.g. {missing[:5]}')
        if len(extra) > slack:
            failures.append(f'{role} region at ({x},{y}): {len(extra)} empty cells show ink, e.g. {extra[:5]}')
    return failures


def cell_shapes(png, x, y, w, h, cw, ch, background, k=5):
    """A k x k ink signature per cell (same thresholding as cell_ink)."""
    cols, rows = int(w // cw), int(h // ch)
    if cols <= 0 or rows <= 0:
        return {}
    out = magick(png, '-crop', f'{int(cols * cw)}x{int(rows * ch)}+{int(x)}+{int(y)}', '+repage',
                 '-fuzz', '12%', '-fill', 'black', '-opaque', f'#{background}',
                 '-colorspace', 'gray', '-threshold', '25%',
                 '-filter', 'box', '-resize', f'{cols * k}x{rows * k}!', '-depth', '16', 'txt:-')
    shapes = {}
    for line in out.splitlines()[1:]:
        pos, rest = line.split(':', 1)
        px, py = map(int, pos.split(','))
        v = float(rest.split('(')[1].split(')')[0].split(',')[0]) / 65535.0
        shapes.setdefault((py // k, px // k), [0.0] * (k * k))[(py % k) * k + px % k] = v
    return shapes


SHAPE_TOLERANCE = 0.12   # mean |difference| from the character's median shape


def check_identity(png, glyphs, dy=0, cw=10.0, ch=25.0):
    """Every occurrence of the same character in the same colours must look
    the same on screen. A frame showing other text at the dumped cells --
    stale scrolling, swapped digits, rows shifted by one line -- puts
    different shapes into one character's group and fails."""
    failures = []
    for (role, x, y, w, h), gs in regions(glyphs).items():
        background = (gs[0]['background'] if role == 'ModeLine' else '#000000').lstrip('#')
        shapes = cell_shapes(png, x, y - dy, w, h, cw, ch, background)
        groups = {}
        for g in gs:
            cell = (int(round((g['y'] - y) / ch)), int(round((g['x'] - x) / cw)))
            if g['char'].strip() and cell in shapes and not masked(x + cell[1] * cw, y + cell[0] * ch):
                groups.setdefault((g['char'], g['foreground']), []).append((cell, shapes[cell]))
        bad = []
        for (char, _), members in groups.items():
            if len(members) < 2:
                continue
            median = [sorted(v[i] for _, v in members)[len(members) // 2] for i in range(len(members[0][1]))]
            for cell, v in members:
                if sum(abs(a - b) for a, b in zip(v, median)) / len(v) > SHAPE_TOLERANCE:
                    bad.append((char, cell))
        # The cursor (a filled or hollow box over one cell) changes that
        # cell's shape, and in a two-member group the median may pick it, so
        # text areas get two cells of slack, mode lines none.
        if len(bad) > (2 if role == 'Text' else 0):
            failures.append(f'{role} region at ({x},{y}): {len(bad)} cells do not look like their dumped character, e.g. {bad[:5]}')
    return failures


def vertical_profiles(png, x, y, w, ch, background, cw=10.0):
    """Ink per pixel row (0..ch-1) for each cell of a one-row region."""
    cols = int(w // cw)
    out = magick(png, '-crop', f'{int(cols * cw)}x{int(ch)}+{int(x)}+{int(y)}', '+repage',
                 '-fuzz', '12%', '-fill', 'black', '-opaque', f'#{background}',
                 '-colorspace', 'gray', '-threshold', '25%',
                 '-filter', 'box', '-resize', f'{cols}x{int(ch)}!', '-depth', '16', 'txt:-')
    prof = {}
    for line in out.splitlines()[1:]:
        pos, rest = line.split(':', 1)
        c, r = map(int, pos.split(','))
        prof.setdefault(c, [0.0] * int(ch))[r] = float(rest.split('(')[1].split(')')[0].split(',')[0]) / 65535.0
    return prof


def flag_shape_ok(char, rows):
    """Geometry of the mode line's flag characters, independent of any other
    cell: '-' is one thin bar at mid height, '*' sits in the upper half and
    spans several rows, '%' spans most of the cell, ':' is two dots with a
    gap. Swapping '*' and '-' (the modified flag) changes the class."""
    lit = [i for i, v in enumerate(rows) if v > 0.1]
    if not lit:
        return False
    top, bottom, extent = lit[0], lit[-1], lit[-1] - lit[0] + 1
    if char == '-':
        return extent <= 3 and 8 <= top <= 16
    if char == '*':
        return extent >= 4 and top <= 8 and bottom <= 14
    if char == '%':
        return extent >= 10
    if char == ':':
        return extent >= 6 and any(rows[i] <= 0.1 for i in range(top, bottom + 1))
    return True


def check_flags(png, glyphs, dy=0):
    failures = []
    for (role, x, y, w, h), gs in regions(glyphs).items():
        if role != 'ModeLine':
            continue
        prof = vertical_profiles(png, x, y - dy, w, h, gs[0]['background'].lstrip('#'))
        bad = [(g['char'], int((g['x'] - x) // 10)) for g in gs if g['char'] in '-*%:'
               and not flag_shape_ok(g['char'], prof.get(int(round((g['x'] - x) / 10)), []))]
        if bad:
            failures.append(f'mode line at ({x},{y}): flag cells do not have their character\'s shape: {bad[:6]}')
    return failures


# Independent character references: the GUI's font, rasterised by
# ImageMagick/FreeType, never by the program under test. The scene log names
# it ("romacs: font family=Iosevka points=14 pixels=18.666666").
REF_FONT_FAMILY, REF_PIXELS = 'Iosevka', '18.6667'
REF_W, REF_H, REF_X, REF_BASE = 30, 55, 10, 35   # canvas and pen origin
# Canvas row of a screen cell's top, measured once on live/default/00-startup:
# mean NCC of 30 mode-line letters 0.986 at rows 15-17, 0.70 at 14 and 18.
REF_TOP = 16
REF_OWN = 0.90
# Rivals: every character a substitute could plausibly be drawn from --
# ASCII, Latin-1, Greek, general punctuation (dashes, bullets), arrows, math
# operators (minus, asterisk operator), box drawing. The cell's BEST match
# among all of them must be its own character or a homoglyph of it: a
# character whose reference matches the own reference at >= REF_SAME (Greek
# mu and the micro sign) cannot show different content.
REF_SAME = 0.97
REF_CANDIDATES = [chr(i) for r in ((33, 127), (161, 256), (0x370, 0x400), (0x2010, 0x205f), (0x2190, 0x2200),
                                   (0x2200, 0x2300), (0x2500, 0x2580)) for i in range(*r)]
CURSORS = None    # the step's published cursor cells, set by the caller; None exempts nothing
_refs, _windows, _verdicts, _same = {}, {}, {}, {}


def _gray(args, w, h):
    raw = subprocess.check_output(['magick', *map(str, args), '-colorspace', 'gray', '-depth', '8', 'gray:-'])
    assert len(raw) == w * h, (len(raw), w, h)
    return [[raw[r * w + c] / 255.0 for c in range(w)] for r in range(h)]


def ref_font():
    return subprocess.check_output(['fc-match', '-f', '%{file}', REF_FONT_FAMILY], text=True).strip()


def reference(char):
    """CHAR on a REF_W x REF_H canvas. -annotate interprets '\\', '%' and a
    leading '@', so those are escaped (an unescaped backslash drew nothing)."""
    if char not in _refs:
        text = char.replace('\\', '\\\\').replace('%', '%%')
        if text.startswith('@'):
            text = '\\' + text
        _refs[char] = _gray(['-size', f'{REF_W}x{REF_H}', 'xc:black', '-font', ref_font(), '-pointsize', REF_PIXELS,
                             '-density', '72', '-fill', 'white', '-annotate', f'+{REF_X}+{REF_BASE}', text], REF_W, REF_H)
    return _refs[char]


def _unit(values):
    """Zero-mean, unit-norm copy (None for a flat image)."""
    m = sum(values) / len(values)
    d = [v - m for v in values]
    n = sum(v * v for v in d) ** 0.5
    return [v / n for v in d] if n > 0 else None


def _window(r, dy, dx, cw=10, ch=25):
    return [v for row in r[REF_TOP + dy:REF_TOP + dy + ch] for v in row[REF_X + dx:REF_X + dx + cw]]


def windows(char):
    """CHAR's reference at the cell position with one pixel of jitter each way."""
    if char not in _windows:
        r = reference(char)
        _windows[char] = [u for u in (_unit(_window(r, dy, dx)) for dy in (-1, 0, 1) for dx in (-1, 0, 1)) if u]
    return _windows[char]


def ref_score(cell, char):
    """Best normalised correlation of a screen cell (a flat list or rows) with CHAR's reference."""
    flat = cell if not cell or not isinstance(cell[0], list) else [v for row in cell for v in row]
    u = _unit(flat)
    if u is None:
        return 0.0
    return max((sum(map(float.__mul__, u, w)) for w in windows(char)), default=0.0)


def same_glyph(a, b):
    """True when the two references cannot be told apart on screen."""
    if (a, b) not in _same:
        _same[(a, b)] = _same[(b, a)] = ref_score(_window(reference(a), 0, 0), b) >= REF_SAME
    return _same[(a, b)]


def check_reference(png, glyphs, dy=0, cw=10, ch=25):
    """Every non-space glyph cell, in mode lines AND text areas, must match its
    dumped character's INDEPENDENT reference (score >= REF_OWN), and no
    distinguishable rival may match it as well or better. Unlike
    check_identity this does not compare the screenshot with itself, so a
    consistent substitution (every '*' drawn as '-' or '+', '-' as a dot or an
    en dash, text '0' as '8') fails. Exempt: the published cursor cell of
    each window (the cursor box covers it), and cells under the notification
    mask."""
    failures = []
    cursors = {(float(c['x']), float(c['y'])) for c in (CURSORS or [])}
    for (role, x, y, w, h), gs in regions(glyphs).items():
        cols, rows = int(w // cw), int(h // ch)
        bg = gs[0]['background'] if role == 'ModeLine' else '#000000'
        # A viewport, not -crop: a region shifted past the image edge (the
        # shifted-300px mutant) reads black instead of a smaller image.
        img = _gray([png, '-set', 'option:distort:viewport', f'{cols * cw}x{rows * ch}{int(x):+d}{int(y - dy):+d}',
                     '-virtual-pixel', 'black', '-filter', 'point', '-distort', 'SRT', '0', '+repage',
                     '-fuzz', '12%', '-fill', 'black', '-opaque', bg], cols * cw, rows * ch)
        bad = []
        for g in gs:
            c, r = int(round((g['x'] - x) / cw)), int(round((g['y'] - y) / ch))
            if (not g['char'].strip() or c >= cols or r >= rows or masked(x + c * cw, y + r * ch)
                    or (role == 'Text' and (float(g['x']), float(g['y'])) in cursors)):
                continue
            cell = [v for row in img[r * ch:(r + 1) * ch] for v in row[c * cw:(c + 1) * cw]]
            key = (g['char'], bytes(int(v * 255) for v in cell))
            if key not in _verdicts:
                own, rival = ref_score(cell, g['char']), (0.0, '')
                if own >= REF_OWN:
                    for k in REF_CANDIDATES:
                        if k != g['char']:
                            score = ref_score(cell, k)
                            if score >= own and not same_glyph(g['char'], k):
                                rival = (score, k)
                                break
                _verdicts[key] = (None if own >= REF_OWN and not rival[1]
                                  else (round(own, 2), rival[1], round(rival[0], 2)))
            if _verdicts[key]:
                bad.append((g['char'], r, c, _verdicts[key]))
        if bad:
            failures.append(f'{role} region at ({x},{y}): {len(bad)} cells do not match their character\'s font '
                            f'reference, e.g. (char, row, column, (own, a rival at least as good, its score)) {bad[:4]}')
    return failures


def check_step(png, bands, glyphs, dy=0):
    """Failures for a screenshot against a step's dumped bands and glyphs.
    dy shifts every sampled region (a wrong-geometry mutant)."""
    global MASK_ON
    MASK_ON = notifications_present(png)
    failures = []
    for i, b in enumerate(bands):
        x, y, w, h = b['rect']
        bg = b['backgrounds'][0]
        strip = mean_colour(png, x + w - 60, y + 2 - dy, 40, max(h - 4, 1))
        if not close(strip, bg):
            failures.append(f'band {i} background {strip} != {bg}')
    return (failures + check_cells(png, glyphs, dy) + check_identity(png, glyphs, dy) + check_flags(png, glyphs, dy)
            + check_reference(png, glyphs, dy)
            + check_ocr(png, glyphs, dy)[0])


def window_top(bands, b):
    x, y, w, _ = b['rect']
    tops = [o['rect'][1] + o['rect'][3] for o in bands
            if o is not b and o['rect'][1] + o['rect'][3] <= y and o['rect'][0] < x + w and x < o['rect'][0] + o['rect'][2]]
    return max(tops, default=0.0)


# --- Independent content anchor: OCR ---------------------------------------
# Cell ink and shape consistency cannot tell a '1' from a '2' if every '1'
# on screen was consistently replaced. Tesseract reads each region from the
# compositor pixels alone and the result is compared, row by row in order,
# with the text the GUI dumped.
import difflib, os, tempfile

OCR_ROW_MATCH = 0.9     # fraction of a text region's rows read exactly
OCR_CHAR_ERRORS = 0.015 # genuine screenshots read at <= 1.1% after canon()


def ocr_region(png, x, y, w, h, background, scale=3, boost=False):
    with tempfile.TemporaryDirectory() as d:
        img = os.path.join(d, 'r.png')
        mask = []
        mx, my, mw, mh = NOTIFICATION_MASK
        if MASK_ON and x + w > mx and y < my + mh:
            mask = ['-fill', 'black', '-draw', f'rectangle {max(mx - x, 0)},0 {w},{max(my + mh - y, 0)}']
        magick(png, '-crop', f'{int(w)}x{int(h)}+{int(x)}+{int(y)}', '+repage',
               '-fuzz', '12%', '-fill', 'black', '-opaque', f'#{background}', *mask,
               '-colorspace', 'gray', *(['-level', '0%,50%'] if boost else []), '-negate', '-resize', f'{scale * 100}%', img)
        out = subprocess.run(['tesseract', img, '-', '--psm', '6'], capture_output=True, text=True).stdout
    return [' '.join(l.split()) for l in out.splitlines() if l.strip()]


def dumped_rows(gs, x, y, w, ch=25.0, cw=10.0):
    """Dumped text per row, skipping rows that cross the notification mask
    (half a row cannot be compared with OCR)."""
    mx, my, mw, mh = NOTIFICATION_MASK
    rows = {}
    for g in gs:
        rows.setdefault(int(round((g['y'] - y) / ch)), {})[int(round((g['x'] - x) / cw))] = g['char']
    lines = []
    for r in sorted(rows):
        if MASK_ON and x + w > mx and y + r * ch < my + mh:
            continue
        cells = rows[r]
        line = ''.join(cells.get(c, ' ') for c in range(max(cells) + 1))
        if line.strip():
            lines.append(' '.join(line.split()))
    return lines


def words(line):
    import re
    return re.findall(r'[A-Za-z0-9]{2,}', line)


def digits(line):
    import re
    return re.findall(r'[0-9]+', line)


def position(line):
    """The mode line's position claims: All/Top/Bottom/NN% and L<line>."""
    import re
    return re.findall(r'\bL\d+|\d+%|\bAll\b|\bTop\b|\bBottom\b', line)


# Tesseract's known misreadings of this font, applied to both sides: the
# dotted zero reads as 8 or B, a stray 1 appears before l/i, and Greek reads
# as Latin homoglyphs. A substitution inside one of these classes (0<->8)
# would go unnoticed; any other wrong digit or letter is caught.
GREEK = str.maketrans({'κ': 'k', 'ό': 'o', 'ο': 'o', 'σ': 'o', 'μ': 'p', 'ε': 'e', 'λ': 'A', 'Ο': 'O'})


def canon(line):
    import re
    line = line.translate(GREEK)
    line = re.sub(r'1(?=[li])', '', line)
    return re.sub(r'[08BO]', '0', line)


def same_row(a, b):
    """OCR row b reads dumped row a: every number exactly and nearly every
    character, modulo Tesseract's known confusions in this font."""
    a, b = canon(a), canon(b)
    return digits(a) == digits(b) and difflib.SequenceMatcher(None, a, b).ratio() >= 0.9


def check_ocr(png, glyphs, dy=0):
    failures, report = [], []
    for (role, x, y, w, h), gs in regions(glyphs).items():
        background = (gs[0]['background'] if role == 'ModeLine' else '#000000').lstrip('#')
        want = dumped_rows(gs, x, y, w)
        if not want:
            continue
        # Two readings: plain, and with dim faces (comments) brightened.
        if role == 'Text':
            # OCR only the rows being compared: from the first to the last
            # dumped text row, and below the notification mask when it is on.
            mx, my, mw, mh = NOTIFICATION_MASK
            skip = max(0.0, (my + mh - y) // 25 * 25 + 25) if MASK_ON and x + w > mx and y < my + mh else 0.0
            ys = [g['y'] for g in gs if g['char'].strip() and g['y'] >= y + skip]
            top, bottom = min(ys), max(ys) + 25
            reads = [ocr_region(png, x, top - dy, w, bottom - top, background, boost=b) for b in (False, True)]
        else:
            reads = [ocr_region(png, x, y - dy, w, h, background, boost=b) for b in (False, True)]
        if role == 'ModeLine':
            a = ' '.join(want)
            ok, best = False, None
            for got in reads:
                b = ' '.join(got)
                seen = words(b)
                recall = sum(1 for t in words(a) if t in seen) / max(len(words(a)), 1)
                if position(canon(a)) == position(canon(b)) and recall >= 0.8:
                    ok = True
                best = best or b
            report.append((role, x, y, ok))
            if not ok:
                failures.append(f'mode line at ({x},{y}) reads {best!r}, dumped {a!r}')
            continue
        # Align rows in order; a row counts if either reading matches it.
        hits = set()
        for got in reads:
            j = 0
            for n, a in enumerate(want):
                for k in range(j, min(j + 3, len(got))):
                    if same_row(a, got[k]):
                        hits.add(n)
                        j = k + 1
                        break
        frac = len(hits) / len(want)
        # And across the whole region, few character errors: a sparse but
        # systematic substitution (every 'r' drawn as 'o') changes too few
        # characters per row to fail a row, but too many in total.
        a = canon('\n'.join(want))
        errors = min(len(a) - sum(m.size for m in difflib.SequenceMatcher(None, a, canon('\n'.join(got)), autojunk=False)
                                  .get_matching_blocks()) for got in reads)
        rate = errors / max(len(a), 1)
        report.append((role, x, y, round(frac, 2), round(rate, 3)))
        if frac < OCR_ROW_MATCH:
            failures.append(f'text at ({x},{y}): {len(hits)}/{len(want)} rows read as dumped')
        if rate > OCR_CHAR_ERRORS:
            failures.append(f'text at ({x},{y}): {errors} of {len(a)} characters read differently from the dump')
    return failures, report
