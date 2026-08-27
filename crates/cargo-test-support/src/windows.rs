//! Windows-specific test support.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::process::{Command, ExitStatus, Stdio};

use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
use windows_sys::Win32::System::Console::{
    AllocConsole, CONSOLE_SCREEN_BUFFER_INFO, CONSOLE_TEXTMODE_BUFFER, COORD,
    CreateConsoleScreenBuffer, ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    ENABLE_WRAP_AT_EOL_OUTPUT, FreeConsole, GetConsoleCP, GetConsoleMode,
    GetConsoleScreenBufferInfo, ReadConsoleOutputCharacterW, SetConsoleMode,
    SetConsoleScreenBufferSize,
};

const SCREEN_WIDTH: i16 = 120;
const MIN_SCREEN_HEIGHT: i16 = 100;

/// The result of running a command with its standard error attached to a console.
pub struct ConsoleOutput {
    /// The command's exit status.
    pub status: ExitStatus,
    /// The characters rendered in the console screen buffer.
    pub screen: String,
}

/// Runs a command with standard error attached to a real Windows console screen buffer.
///
/// Unlike a pipe, the returned screen reflects how Windows processed control characters such as
/// carriage returns.
pub fn run_in_console(mut command: Command) -> io::Result<ConsoleOutput> {
    let _console = ConsoleAttachment::new()?;
    let screen = ScreenBuffer::new()?;
    command.stderr(Stdio::from(screen.handle.try_clone()?));

    let status = command.status()?;
    let screen = screen.contents()?;

    Ok(ConsoleOutput { status, screen })
}

struct ConsoleAttachment {
    allocated: bool,
}

impl ConsoleAttachment {
    fn new() -> io::Result<Self> {
        // SAFETY: `GetConsoleCP` has no preconditions.
        let attached = unsafe { GetConsoleCP() } != 0;
        let allocated = if attached {
            false
        } else {
            // SAFETY: The process is not currently attached to a console.
            check(unsafe { AllocConsole() })?;
            true
        };
        Ok(Self { allocated })
    }
}

impl Drop for ConsoleAttachment {
    fn drop(&mut self) {
        if self.allocated {
            // SAFETY: This process allocated the console in `ConsoleAttachment::new`.
            unsafe { FreeConsole() };
        }
    }
}

struct ScreenBuffer {
    handle: OwnedHandle,
}

impl ScreenBuffer {
    fn new() -> io::Result<Self> {
        // SAFETY: Null optional parameters are permitted and the remaining values are valid flags.
        let raw_handle = unsafe {
            CreateConsoleScreenBuffer(
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                CONSOLE_TEXTMODE_BUFFER,
                std::ptr::null(),
            )
        };
        if raw_handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `CreateConsoleScreenBuffer` returned a new owned handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw_handle as _) };
        let screen = Self { handle };

        let mut mode = 0;
        // SAFETY: `screen.handle` is a valid console screen buffer handle and `mode` is writable.
        check(unsafe { GetConsoleMode(screen.raw_handle(), &mut mode) })?;
        // SAFETY: `screen.handle` is valid and the mode consists of documented output flags.
        check(unsafe {
            SetConsoleMode(
                screen.raw_handle(),
                mode | ENABLE_PROCESSED_OUTPUT
                    | ENABLE_WRAP_AT_EOL_OUTPUT
                    | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            )
        })?;

        let info = screen.info()?;
        let size = COORD {
            X: SCREEN_WIDTH,
            Y: info.dwSize.Y.max(MIN_SCREEN_HEIGHT),
        };
        // SAFETY: `screen.handle` is valid and `size` is large enough for the console window.
        check(unsafe { SetConsoleScreenBufferSize(screen.raw_handle(), size) })?;

        Ok(screen)
    }

    fn contents(&self) -> io::Result<String> {
        let info = self.info()?;
        let width = usize::try_from(info.dwSize.X).unwrap();
        let height = usize::try_from(info.dwCursorPosition.Y + 1).unwrap();
        let mut characters = vec![0; width * height];
        let mut characters_read = 0;

        // SAFETY: `self.handle` is valid and `characters` has room for the requested character
        // count. `characters_read` is writable.
        check(unsafe {
            ReadConsoleOutputCharacterW(
                self.raw_handle(),
                characters.as_mut_ptr(),
                u32::try_from(characters.len()).unwrap(),
                COORD { X: 0, Y: 0 },
                &mut characters_read,
            )
        })?;
        characters.truncate(characters_read as usize);

        let mut lines = characters
            .chunks(width)
            .map(String::from_utf16_lossy)
            .map(|line| line.trim_end().to_owned())
            .collect::<Vec<_>>();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        Ok(lines.join("\n"))
    }

    fn info(&self) -> io::Result<CONSOLE_SCREEN_BUFFER_INFO> {
        let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
        // SAFETY: `self.handle` is valid and `info` is writable.
        check(unsafe { GetConsoleScreenBufferInfo(self.raw_handle(), &mut info) })?;
        Ok(info)
    }

    fn raw_handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.handle.as_raw_handle() as _
    }
}

fn check(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
