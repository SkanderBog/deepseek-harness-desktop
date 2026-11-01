import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

assert sys.platform == "win32", "this check requires Windows and NSIS"
nsis = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(os.environ["LOCALAPPDATA"]) / "tauri/NSIS"
arch = sys.argv[2] if len(sys.argv) > 2 else "x64"
assert arch in ("x64", "arm64", "x86")
root = Path(tempfile.mkdtemp(prefix="dsh-installer-shutdown-"))
children = []


def run(program, *args):
    result = subprocess.run([str(program), *map(str, args)], capture_output=True, text=True, timeout=30, creationflags=subprocess.CREATE_NO_WINDOW)
    assert result.returncode == 0, result.stdout + result.stderr


def start(binary, dll):
    child = subprocess.Popen([str(binary), str(dll)], stdout=subprocess.PIPE, text=True, creationflags=subprocess.CREATE_NO_WINDOW)
    children.append(child)
    assert child.stdout.readline().strip() == "ready", "fixture must load its DLL and create a window"
    return child


try:
    (root / "fixture.rs").write_text(r'''
#![windows_subsystem = "windows"]
use std::{ffi::c_void, ptr, thread, time::Duration};
#[link(name = "kernel32")]
extern "system" { fn LoadLibraryW(name: *const u16) -> *mut c_void; }
#[link(name = "user32")]
extern "system" {
  fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: *mut c_void, menu: *mut c_void, instance: *mut c_void, param: *mut c_void) -> *mut c_void;
  fn SetWindowLongPtrW(window: *mut c_void, index: i32, value: isize) -> isize;
  fn DefWindowProcW(window: *mut c_void, message: u32, wparam: usize, lparam: isize) -> isize;
  fn GetMessageW(message: *mut c_void, window: *mut c_void, min: u32, max: u32) -> i32;
  fn DispatchMessageW(message: *const c_void) -> isize;
}
unsafe extern "system" fn procedure(window: *mut c_void, message: u32, wparam: usize, lparam: isize) -> isize {
  if message == 0x11 { return 1; }
  if message == 0x10 || message == 0x16 && wparam != 0 { std::process::exit(0); }
  DefWindowProcW(window, message, wparam, lparam)
}
fn main() {
  let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
  unsafe {
    assert!(!LoadLibraryW(wide(&std::env::args().nth(1).unwrap()).as_ptr()).is_null());
    let window = CreateWindowExW(0, wide("STATIC").as_ptr(), wide("isolated installer fixture").as_ptr(), 0, 0, 0, 1, 1, ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
    assert!(!window.is_null());
    assert_ne!(SetWindowLongPtrW(window, -4, procedure as *const () as isize), 0);
    thread::spawn(|| { thread::sleep(Duration::from_secs(30)); std::process::exit(2); });
    println!("ready");
    let mut message = [0usize; 8];
    while GetMessageW(message.as_mut_ptr().cast(), ptr::null_mut(), 0, 0) > 0 { DispatchMessageW(message.as_ptr().cast()); }
  }
}
''', encoding="utf-8")
    run("rustc", root / "fixture.rs", "-o", root / "host.exe")
    runtime = root / "runtime" / f"win32-{'ia32' if arch == 'x86' else arch}"
    unrelated = root / "unrelated"
    system_dll = Path(os.environ["SystemRoot"]) / "System32/version.dll"
    for directory in (runtime, unrelated):
        directory.mkdir(parents=True)
        shutil.copyfile(root / "host.exe", directory / "branded.exe")
        shutil.copyfile(system_dll, directory / "chrome_elf.dll")
    sentinel = start(unrelated / "branded.exe", unrelated / "chrome_elf.dll")
    host = start(root / "host.exe", unrelated / "chrome_elf.dll")
    electron = start(runtime / "branded.exe", runtime / "chrome_elf.dll")
    source = (Path(__file__).resolve().parents[1] / "src-tauri/templates/installer.nsi").read_text(encoding="utf-8")
    begin = source.index("!macroundef RestartManager_RegisterFile")
    macro = source[begin:source.index("!macroend", begin) + len("!macroend")]
    harness = r'''
Unicode true
!include "Win\RestartManager.nsh"
!define ARCH @ARCH@
@MACRO@
OutFile "@ROOT@\check.exe"
RequestExecutionLevel user
SilentInstall silent
Section
  StrCpy $INSTDIR "@ROOT@"
  !insertmacro RestartManager_StartSession $R0
  !insertmacro RestartManager_RegisterFile $R0 "$INSTDIR\host.exe"
  IntCmp $0 0 +3
    SetErrorLevel 1
    Quit
  System::Call 'RSTRTMGR::RmGetList(p R0, *i .r1, *i .r2, p 0, *i .r3) i .r0'
  FileOpen $R5 "$INSTDIR\count.txt" w
  FileWrite $R5 "$1"
  FileClose $R5
  System::Call 'RSTRTMGR::RmShutdown(p R0, i 1, p 0) i .r0'
  StrCpy $R4 $0
  !insertmacro RestartManager_EndSession $R0
  SetErrorLevel $R4
SectionEnd
'''.replace("@MACRO@", macro).replace("@ROOT@", str(root)).replace("@ARCH@", arch)
    (root / "check.nsi").write_text(harness, encoding="utf-8")
    run(nsis / "makensis.exe", "/V2", root / "check.nsi")
    run(root / "check.exe", "/S")
    assert (root / "count.txt").read_text() == "2", "one session must include the host and the old Electron DLL owner"
    for child in (host, electron):
        assert child.wait(timeout=5) == 0, "registered process must exit"
    assert sentinel.poll() is None, "a separate installation must keep running"
    shutil.copyfile(system_dll, runtime / "chrome_elf.dll")
    print("installer shutdown: PASS (old host, old Electron, one Restart Manager session, unrelated installation preserved)")
finally:
    for child in children:
        if child.poll() is None:
            child.kill()
        child.wait(timeout=5)
        child.stdout.close()
    shutil.rmtree(root)
