#!/usr/bin/env python3
"""Build-only shadow workspace; never edit the parent-owned workspace manifest."""
from pathlib import Path
import argparse
import shutil

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--red-layout', action='store_true', help='compile the original executable no-op layout stub')
args = parser.parse_args()
root = Path(__file__).resolve().parents[3]
shadow = root / 'crates/romacs-vm-gui' / ('.verify-red' if args.red_layout else '.verify')
shadow.mkdir(exist_ok=True)
(shadow / 'crates').mkdir(exist_ok=True)
for entry in root.iterdir():
    if entry.name in ('Cargo.toml', 'Cargo.lock', 'crates', '.git'):
        continue
    target = shadow / entry.name
    if not target.exists():
        target.symlink_to(entry, target_is_directory=entry.is_dir())
for entry in (root / 'crates').iterdir():
    target = shadow / 'crates' / entry.name
    if target.exists():
        continue
    if args.red_layout and entry.name == 'romacs-layout2':
        target.mkdir()
        shutil.copy2(entry / 'Cargo.toml', target / 'Cargo.toml')
        shutil.copytree(entry / 'tests', target / 'tests')
        (target / 'src').mkdir()
        (target / 'src/lib.rs').write_text('use romacs_display_protocol::FrameGlyphBuffer;\n'
            'use romacs_layout_input::WindowSnapshot;\n'
            'pub fn layout(_: &WindowSnapshot, _:u64)->Result<FrameGlyphBuffer,String> '
            '{Ok(FrameGlyphBuffer::with_size(1.0,1.0))}\n')
    else:
        target.symlink_to(entry, target_is_directory=True)
manifest = (root / 'Cargo.toml').read_text()
missing = [f'    "crates/{name}",' for name in ('romacs-layout2', 'romacs-vm-gui')
           if f'"crates/{name}"' not in manifest]
manifest = manifest.replace('members = [', 'members = [\n' + '\n'.join(missing), 1)
(shadow / 'Cargo.toml').write_text(manifest)
# A symlink here lets Cargo accidentally update the integrating parent's lock.
lock = shadow / 'Cargo.lock'
if lock.is_symlink():
    lock.unlink()
if not lock.exists():
    shutil.copy2(root / 'Cargo.lock', lock)
print(shadow / 'Cargo.toml')
