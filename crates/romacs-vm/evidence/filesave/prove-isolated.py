#!/usr/bin/env python3
"""Focused integration preview in a private copy, never patch shared files.

The parent still must install and validate these wrappers in the real lane.
"""
import os as _portable_os
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import hashlib, json, os, pathlib, shutil, subprocess, tempfile
ROOT = pathlib.Path(__file__).resolve().parents[4]
HERE = pathlib.Path(__file__).resolve().parent
with tempfile.TemporaryDirectory(prefix='filesave-proof-', dir=str(_PortablePath(_portable_tempfile.gettempdir()) / '')) as d:
    work = pathlib.Path(d)
    archive = subprocess.Popen(['git', 'archive', 'HEAD'], cwd=ROOT, stdout=subprocess.PIPE)
    assert archive.stdout is not None
    subprocess.run(['tar', '-xf', '-', '-C', str(work)], stdin=archive.stdout, check=True)
    archive.stdout.close()
    if archive.wait(): raise RuntimeError('git archive failed')
    for relative in ['src/prims/filesave.rs', 'tests/filesave.rs']:
        shutil.copy2(ROOT/'crates/romacs-vm'/relative, work/'crates/romacs-vm'/relative)
    fileio = work/'crates/romacs-vm/src/prims/fileio.rs'
    source = fileio.read_text()
    source = source.replace('fn find_file_name_handler(cx:', 'pub(super) fn find_file_name_handler(cx:')
    fileio.write_text(source)
    leaf = work/'crates/romacs-vm/src/prim/leaf/fileio.rs'
    with leaf.open('a') as f: f.write(r'''
impl Leaf<'_> {
    pub fn filesave_access(&self, path: &str, mode: i32) -> std::io::Result<()> {
        let path = std::ffi::CString::new(path).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        if unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), mode, libc::AT_EACCESS) } == 0 { Ok(()) }
        else { Err(std::io::Error::last_os_error()) }
    }
    pub fn filesave_chmod(&self, path: &str, mode: u32, nofollow: bool) -> std::io::Result<()> {
        let path = std::ffi::CString::new(path).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        if unsafe { libc::fchmodat(libc::AT_FDCWD, path.as_ptr(), mode as libc::mode_t, if nofollow { libc::AT_SYMLINK_NOFOLLOW } else { 0 }) } == 0 { Ok(()) }
        else { Err(std::io::Error::last_os_error()) }
    }
    pub fn filesave_rename(&self, from: &str, to: &str, replace: bool) -> std::io::Result<()> {
        if replace { return std::fs::rename(from, to) }
        let from = std::ffi::CString::new(from).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        let to = std::ffi::CString::new(to).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        if unsafe { libc::renameat2(libc::AT_FDCWD, from.as_ptr(), libc::AT_FDCWD, to.as_ptr(), libc::RENAME_NOREPLACE) } == 0 { Ok(()) }
        else { Err(std::io::Error::last_os_error()) }
    }
}
''')
    env = os.environ.copy()
    env['CARGO_TARGET_DIR']=str(_PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'lanes/save-target')
    env['CARGO_BUILD_JOBS']='4'
    env['TMPDIR']=str(_PortablePath(_portable_tempfile.gettempdir()) / '')
    for key in ['GIT_DIR','GIT_WORK_TREE','LC_ALL','GNUPGHOME']: env.pop(key,None)
    run = subprocess.run(['cargo','test','-p','romacs-vm','--test','filesave','--','--nocapture'],cwd=work,env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
    (HERE/'isolated-green.log').write_text(run.stdout)
    manifest = {'base': subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(), 'exit': run.returncode,
        'sha256': {p: hashlib.sha256((ROOT/'crates/romacs-vm'/p).read_bytes()).hexdigest() for p in ['src/prims/filesave.rs','tests/filesave.rs']}}
    (HERE/'isolated-source.json').write_text(json.dumps(manifest,indent=2)+'\n')
    print(run.stdout)
    raise SystemExit(run.returncode)
