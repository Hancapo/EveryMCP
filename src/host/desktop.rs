use serde_json::Value;

const MAX_PIXELS: usize = 64 * 1024 * 1024;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    #[cfg(windows)]
    {
        windows_impl::execute(name, args)
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        Err(format!("{name} is available only on Windows."))
    }
}

/// Decodes a packed device-independent bitmap (CF_DIB clipboard payload) into RGBA pixels.
#[cfg_attr(not(windows), allow(dead_code))]
fn dib_to_rgba(data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let u16_at = |o: usize| u16::from_le_bytes([data[o], data[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
    if data.len() < 40 {
        return Err("Clipboard bitmap header is truncated.".into());
    }
    let header = u32_at(0) as usize;
    let width = u32_at(4) as i32;
    let height = u32_at(8) as i32;
    let bits = u16_at(14);
    let compression = u32_at(16);
    let colors = u32_at(32) as usize;
    if header < 40 || width <= 0 || height == 0 {
        return Err("Clipboard bitmap has an invalid header.".into());
    }
    // BI_RGB, or BI_BITFIELDS with the standard BGRA masks for 32-bit images.
    if !matches!((bits, compression), (24, 0) | (32, 0) | (32, 3)) {
        return Err(format!(
            "Unsupported clipboard bitmap: {bits} bits per pixel, compression {compression}."
        ));
    }
    let masks = if compression == 3 && header == 40 {
        12
    } else {
        0
    };
    let offset = header + masks + colors * 4;
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    if w * h > MAX_PIXELS {
        return Err("Clipboard bitmap exceeds 64 megapixels.".into());
    }
    let bytes_per_pixel = bits as usize / 8;
    let stride = (w * bytes_per_pixel + 3) & !3;
    if data.len() < offset + stride * h {
        return Err("Clipboard bitmap pixel data is truncated.".into());
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        let source = if height > 0 { h - 1 - row } else { row };
        let line = &data[offset + source * stride..][..w * bytes_per_pixel];
        for pixel in line.chunks_exact(bytes_per_pixel) {
            let alpha = if bytes_per_pixel == 4 { pixel[3] } else { 255 };
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], alpha]);
        }
    }
    // Most producers leave the alpha channel of 32-bit DIBs zeroed; treat that as opaque.
    if bytes_per_pixel == 4 && rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
        rgba.as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|p| p[3] = 255);
    }
    Ok((w as u32, h as u32, rgba))
}

#[cfg(windows)]
mod windows_impl {
    use super::super::{files, io_error, optional_bool, optional_str, optional_u64, required_str};
    use super::{MAX_PIXELS, dib_to_rgba};
    use image::{ImageFormat, RgbaImage};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::path::Path;
    use std::sync::Once;
    use std::time::Duration;
    use sysinfo::{ProcessesToUpdate, System};
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Dwm::{
        DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
    };
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
        CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC,
        SRCCOPY, SelectObject,
    };
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
        GetClipboardFormatNameW, GetClipboardSequenceNumber, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
    };
    use windows::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetForegroundWindow, GetSystemMetrics, GetWindowRect,
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
        IsWindowVisible, PW_RENDERFULLCONTENT, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
        SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    };
    use windows::core::BOOL;

    pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
        // Report physical pixels so window rectangles and captures agree on scaled displays.
        static DPI_AWARE: Once = Once::new();
        DPI_AWARE.call_once(|| unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        });
        match name {
            "window_list" => window_list(args),
            "screen_capture" => screen_capture(args),
            "clipboard_get" => clipboard_get(args),
            "clipboard_set" => clipboard_set(args),
            _ => unreachable!(),
        }
    }

    #[derive(Clone, Copy)]
    struct Rect {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }

    impl Rect {
        fn from_win(r: RECT) -> Self {
            Rect {
                x: r.left,
                y: r.top,
                width: r.right - r.left,
                height: r.bottom - r.top,
            }
        }

        fn intersect(self, other: Rect) -> Option<Rect> {
            let left = self.x.max(other.x);
            let top = self.y.max(other.y);
            let right = (self.x + self.width).min(other.x + other.width);
            let bottom = (self.y + self.height).min(other.y + other.height);
            (right > left && bottom > top).then(|| Rect {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            })
        }

        fn to_json(self) -> Value {
            json!({"x":self.x,"y":self.y,"width":self.width,"height":self.height})
        }
    }

    fn hex_handle(hwnd: HWND) -> String {
        format!("0x{:X}", hwnd.0 as usize)
    }

    fn parse_handle(text: &str) -> Result<HWND, String> {
        let text = text.trim();
        let value = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            Some(hex) => usize::from_str_radix(hex, 16),
            None => text.parse(),
        }
        .map_err(|_| "'hwnd' must be a decimal or 0x hexadecimal window handle.".to_string())?;
        let hwnd = HWND(value as *mut c_void);
        if value == 0 || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return Err(format!("'{text}' is not a valid window handle."));
        }
        Ok(hwnd)
    }

    fn wide_text(buffer: &[u16], length: i32) -> String {
        String::from_utf16_lossy(&buffer[..length.max(0) as usize])
    }

    fn window_title(hwnd: HWND) -> String {
        let length = unsafe { GetWindowTextLengthW(hwnd) }.clamp(0, 32_767) as usize;
        let mut buffer = vec![0u16; length + 1];
        let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
        wide_text(&buffer, copied)
    }

    fn window_class(hwnd: HWND) -> String {
        let mut buffer = [0u16; 256];
        let copied = unsafe { GetClassNameW(hwnd, &mut buffer) };
        wide_text(&buffer, copied)
    }

    fn window_pid(hwnd: HWND) -> u32 {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        pid
    }

    fn is_cloaked(hwnd: HWND) -> bool {
        let mut cloaked = 0u32;
        unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                (&mut cloaked as *mut u32).cast(),
                size_of::<u32>() as u32,
            )
        }
        .is_ok()
            && cloaked != 0
    }

    /// Outer window rectangle, including invisible resize borders.
    fn outer_rect(hwnd: HWND) -> Result<Rect, String> {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect) }
            .map_err(|e| format!("Cannot read window rectangle: {e}"))?;
        Ok(Rect::from_win(rect))
    }

    /// Visible window frame as drawn by DWM; falls back to the outer rectangle.
    fn frame_rect(hwnd: HWND) -> Result<Rect, String> {
        let mut rect = RECT::default();
        let dwm = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                (&mut rect as *mut RECT).cast(),
                size_of::<RECT>() as u32,
            )
        };
        match dwm {
            Ok(()) => Ok(Rect::from_win(rect)),
            Err(_) => outer_rect(hwnd),
        }
    }

    fn virtual_screen() -> Rect {
        unsafe {
            Rect {
                x: GetSystemMetrics(SM_XVIRTUALSCREEN),
                y: GetSystemMetrics(SM_YVIRTUALSCREEN),
                width: GetSystemMetrics(SM_CXVIRTUALSCREEN),
                height: GetSystemMetrics(SM_CYVIRTUALSCREEN),
            }
        }
    }

    fn top_level_windows() -> Result<Vec<HWND>, String> {
        unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
            unsafe { (*(lparam.0 as *mut Vec<HWND>)).push(hwnd) };
            BOOL(1)
        }
        let mut handles = Vec::new();
        unsafe {
            EnumWindows(
                Some(collect),
                LPARAM(&mut handles as *mut Vec<HWND> as isize),
            )
        }
        .map_err(|e| format!("Cannot enumerate windows: {e}"))?;
        Ok(handles)
    }

    fn process_names() -> HashMap<u32, String> {
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::All, true);
        system
            .processes()
            .iter()
            .map(|(pid, process)| (pid.as_u32(), process.name().to_string_lossy().into_owned()))
            .collect()
    }

    fn matches_process(actual: &str, requested: &str) -> bool {
        let actual = actual.to_lowercase();
        actual == requested || actual.strip_suffix(".exe") == Some(requested)
    }

    fn window_list(args: &Value) -> Result<Value, String> {
        let title = optional_str(args, "title")?.map(str::to_lowercase);
        let class = optional_str(args, "className")?.map(str::to_lowercase);
        let pid = optional_u64(args, "pid", 0, u32::MAX as u64)? as u32;
        let name = optional_str(args, "name")?.map(str::to_lowercase);
        let include_hidden = optional_bool(args, "includeHidden", false)?;
        let include_untitled = optional_bool(args, "includeUntitled", false)?;
        let limit = optional_u64(args, "limit", 200, 10_000)? as usize;
        if limit == 0 {
            return Err("'limit' must be at least 1.".into());
        }
        let names = process_names();
        let foreground = unsafe { GetForegroundWindow() };
        let mut windows = Vec::new();
        let mut matched = 0usize;
        for (z_order, hwnd) in top_level_windows()?.into_iter().enumerate() {
            let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
            let cloaked = is_cloaked(hwnd);
            if !include_hidden && (!visible || cloaked) {
                continue;
            }
            let text = window_title(hwnd);
            if !include_untitled && text.is_empty() {
                continue;
            }
            if title
                .as_ref()
                .is_some_and(|t| !text.to_lowercase().contains(t.as_str()))
            {
                continue;
            }
            let class_name = window_class(hwnd);
            if class
                .as_ref()
                .is_some_and(|c| !class_name.to_lowercase().contains(c.as_str()))
            {
                continue;
            }
            let owner = window_pid(hwnd);
            if pid != 0 && owner != pid {
                continue;
            }
            let process = names.get(&owner).cloned();
            if let Some(requested) = &name
                && !process
                    .as_deref()
                    .is_some_and(|p| matches_process(p, requested))
            {
                continue;
            }
            matched += 1;
            if windows.len() < limit {
                windows.push(json!({
                    "hwnd":hex_handle(hwnd),"title":text,"className":class_name,"pid":owner,
                    "processName":process,"rect":frame_rect(hwnd)?.to_json(),"zOrder":z_order,
                    "visible":visible,"cloaked":cloaked,"minimized":unsafe { IsIconic(hwnd) }.as_bool(),
                    "foreground":hwnd == foreground
                }));
            }
        }
        Ok(json!({
            "windows":windows,"matched":matched,"truncated":matched > windows.len(),
            "foregroundHwnd":(!foreground.is_invalid()).then(|| hex_handle(foreground)),
            "virtualScreen":virtual_screen().to_json()
        }))
    }

    enum Source {
        Screen,
        Window(HWND),
    }

    /// Captures `size.width x size.height` pixels as top-down RGBA. Screen sources copy
    /// the desktop at (`size.x`, `size.y`); window sources render the window with PrintWindow.
    fn capture(source: Source, size: Rect) -> Result<Vec<u8>, String> {
        let (width, height) = (size.width, size.height);
        unsafe {
            let screen = GetDC(None);
            if screen.is_invalid() {
                return Err("Cannot open the screen device context.".into());
            }
            let memory = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            if memory.is_invalid() || bitmap.is_invalid() {
                if !bitmap.is_invalid() {
                    let _ = DeleteObject(bitmap.into());
                }
                if !memory.is_invalid() {
                    let _ = DeleteDC(memory);
                }
                ReleaseDC(None, screen);
                return Err("Cannot allocate a capture bitmap.".into());
            }
            let previous = SelectObject(memory, bitmap.into());
            let drawn = match source {
                Source::Screen => BitBlt(
                    memory,
                    0,
                    0,
                    width,
                    height,
                    Some(screen),
                    size.x,
                    size.y,
                    SRCCOPY | CAPTUREBLT,
                )
                .is_ok(),
                Source::Window(hwnd) => {
                    PrintWindow(hwnd, memory, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool()
                }
            };
            SelectObject(memory, previous);
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = vec![0u8; width as usize * height as usize * 4];
            let lines = if drawn {
                GetDIBits(
                    memory,
                    bitmap,
                    0,
                    height as u32,
                    Some(pixels.as_mut_ptr().cast()),
                    &mut info,
                    DIB_RGB_COLORS,
                )
            } else {
                0
            };
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            if !drawn {
                return Err("The screen or window could not be copied.".into());
            }
            if lines != height {
                return Err("Cannot read captured pixels.".into());
            }
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
                pixel[3] = 255;
            }
            Ok(pixels)
        }
    }

    fn crop(pixels: &[u8], width: i32, area: Rect) -> Vec<u8> {
        let mut out = Vec::with_capacity(area.width as usize * area.height as usize * 4);
        for row in area.y..area.y + area.height {
            let start = (row as usize * width as usize + area.x as usize) * 4;
            out.extend_from_slice(&pixels[start..start + area.width as usize * 4]);
        }
        out
    }

    fn optional_i32(args: &Value, key: &str) -> Result<Option<i32>, String> {
        match args.get(key) {
            None => Ok(None),
            Some(value) => value
                .as_i64()
                .and_then(|v| i32::try_from(v).ok())
                .map(Some)
                .ok_or_else(|| format!("'{key}' must be a 32-bit integer.")),
        }
    }

    fn write_png(
        path: &Path,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        overwrite: bool,
    ) -> Result<Value, String> {
        let image =
            RgbaImage::from_raw(width, height, rgba).ok_or("Captured image is malformed.")?;
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, ImageFormat::Png)
            .map_err(|e| io_error("Cannot encode PNG", e))?;
        let bytes = encoded.into_inner();
        files::persist_bytes(path, &bytes, overwrite)?;
        Ok(json!({
            "outputPath":path.to_string_lossy(),"width":width,"height":height,
            "bytesWritten":bytes.len(),"sha256":files::digest_hex(&Sha256::digest(&bytes))
        }))
    }

    fn check_output(path: &Path, overwrite: bool) -> Result<(), String> {
        if path.exists() && !overwrite {
            return Err("Output already exists.".into());
        }
        Ok(())
    }

    fn check_size(area: Rect) -> Result<(), String> {
        if area.width as usize * area.height as usize > MAX_PIXELS {
            return Err("Capture area exceeds 64 megapixels.".into());
        }
        Ok(())
    }

    /// Captures a window's visible frame, either rendered by the window itself
    /// (PrintWindow, works when covered) or copied from the screen as displayed.
    fn capture_window(
        hwnd: HWND,
        from_screen: bool,
        screen: Rect,
    ) -> Result<(Vec<u8>, Rect, Value), String> {
        if unsafe { IsIconic(hwnd) }.as_bool() {
            return Err("The window is minimized; restore it before capturing.".into());
        }
        let window =
            json!({"hwnd":hex_handle(hwnd),"title":window_title(hwnd),"pid":window_pid(hwnd)});
        let frame = frame_rect(hwnd)?;
        if from_screen {
            let area = frame
                .intersect(screen)
                .ok_or("The window is entirely off screen.")?;
            check_size(area)?;
            return Ok((capture(Source::Screen, area)?, area, window));
        }
        // PrintWindow renders the outer rectangle, including invisible resize borders.
        let outer = outer_rect(hwnd)?;
        let inner = frame
            .intersect(outer)
            .ok_or("The window has no visible area.")?;
        check_size(outer)?;
        let rendered = capture(Source::Window(hwnd), outer)?;
        let local = Rect {
            x: inner.x - outer.x,
            y: inner.y - outer.y,
            ..inner
        };
        Ok((crop(&rendered, outer.width, local), inner, window))
    }

    fn screen_capture(args: &Value) -> Result<Value, String> {
        let output = Path::new(required_str(args, "outputPath")?);
        let overwrite = optional_bool(args, "overwrite", false)?;
        check_output(output, overwrite)?;
        let screen = virtual_screen();
        let region = [
            optional_i32(args, "x")?,
            optional_i32(args, "y")?,
            optional_i32(args, "width")?,
            optional_i32(args, "height")?,
        ];
        let hwnd = optional_str(args, "hwnd")?.map(parse_handle).transpose()?;
        let method = optional_str(args, "method")?.unwrap_or("window");
        if !["window", "screen"].contains(&method) {
            return Err("'method' must be window or screen.".into());
        }
        let has_region = region.iter().any(Option::is_some);
        if has_region && hwnd.is_some() {
            return Err("Supply either 'hwnd' or a region, not both.".into());
        }
        let (pixels, area, window) = match hwnd {
            Some(hwnd) => capture_window(hwnd, method == "screen", screen)?,
            None => {
                let requested = match region {
                    [None, None, None, None] => screen,
                    [Some(x), Some(y), Some(width), Some(height)] => {
                        if width <= 0 || height <= 0 {
                            return Err("'width' and 'height' must be positive.".into());
                        }
                        Rect {
                            x,
                            y,
                            width,
                            height,
                        }
                    }
                    _ => return Err("A region needs all of 'x', 'y', 'width' and 'height'.".into()),
                };
                let area = requested
                    .intersect(screen)
                    .ok_or("The region does not overlap any display.")?;
                check_size(area)?;
                (capture(Source::Screen, area)?, area, Value::Null)
            }
        };
        let mut result = write_png(
            output,
            area.width as u32,
            area.height as u32,
            pixels,
            overwrite,
        )?;
        result["rect"] = area.to_json();
        result["source"] = json!(if window.is_null() || method == "screen" {
            "screen"
        } else {
            "window"
        });
        result["window"] = window;
        result["virtualScreen"] = screen.to_json();
        Ok(result)
    }

    /// Holds the clipboard open for the lifetime of the value.
    struct Clipboard;

    impl Clipboard {
        fn open() -> Result<Self, String> {
            for _ in 0..20 {
                if unsafe { OpenClipboard(None) }.is_ok() {
                    return Ok(Clipboard);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err("The clipboard is busy; another application is holding it open.".into())
        }
    }

    impl Drop for Clipboard {
        fn drop(&mut self) {
            let _ = unsafe { CloseClipboard() };
        }
    }

    /// Copies a clipboard format's global memory block, if present.
    fn clipboard_bytes(format: u32) -> Option<Vec<u8>> {
        unsafe {
            let handle = GetClipboardData(format).ok()?;
            let memory = HGLOBAL(handle.0);
            let pointer = GlobalLock(memory) as *const u8;
            if pointer.is_null() {
                return None;
            }
            let data = std::slice::from_raw_parts(pointer, GlobalSize(memory)).to_vec();
            let _ = GlobalUnlock(memory);
            Some(data)
        }
    }

    fn format_name(format: u32) -> String {
        const STANDARD: &[(u32, &str)] = &[
            (1, "CF_TEXT"),
            (2, "CF_BITMAP"),
            (3, "CF_METAFILEPICT"),
            (4, "CF_SYLK"),
            (5, "CF_DIF"),
            (6, "CF_TIFF"),
            (7, "CF_OEMTEXT"),
            (8, "CF_DIB"),
            (9, "CF_PALETTE"),
            (10, "CF_PENDATA"),
            (11, "CF_RIFF"),
            (12, "CF_WAVE"),
            (13, "CF_UNICODETEXT"),
            (14, "CF_ENHMETAFILE"),
            (15, "CF_HDROP"),
            (16, "CF_LOCALE"),
            (17, "CF_DIBV5"),
        ];
        if let Some((_, name)) = STANDARD.iter().find(|(id, _)| *id == format) {
            return (*name).to_owned();
        }
        let mut buffer = [0u16; 256];
        let length = unsafe { GetClipboardFormatNameW(format, &mut buffer) };
        if length > 0 {
            wide_text(&buffer, length)
        } else {
            format!("0x{format:04X}")
        }
    }

    fn clipboard_files() -> Option<Vec<String>> {
        unsafe {
            let handle = GetClipboardData(CF_HDROP.0 as u32).ok()?;
            let drop = HDROP(handle.0);
            let count = DragQueryFileW(drop, u32::MAX, None);
            let mut paths = Vec::with_capacity(count as usize);
            for index in 0..count {
                let length = DragQueryFileW(drop, index, None);
                let mut buffer = vec![0u16; length as usize + 1];
                let copied = DragQueryFileW(drop, index, Some(&mut buffer));
                paths.push(wide_text(&buffer, copied as i32));
            }
            Some(paths)
        }
    }

    fn clipboard_get(args: &Value) -> Result<Value, String> {
        let max_chars = optional_u64(args, "maxChars", 1_000_000, 16 * 1024 * 1024)? as usize;
        let image_output = optional_str(args, "imageOutputPath")?.map(Path::new);
        let overwrite = optional_bool(args, "overwrite", false)?;
        if let Some(path) = image_output {
            check_output(path, overwrite)?;
        }
        let (formats, text, files, dib) = {
            let _clipboard = Clipboard::open()?;
            let mut formats = Vec::new();
            let mut format = 0;
            loop {
                format = unsafe { EnumClipboardFormats(format) };
                if format == 0 || formats.len() >= 256 {
                    break;
                }
                formats.push(json!({"id":format,"name":format_name(format)}));
            }
            let text = clipboard_bytes(CF_UNICODETEXT.0 as u32).map(|bytes| {
                let units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&pair| u16::from_le_bytes(pair))
                    .take_while(|&unit| unit != 0)
                    .collect();
                String::from_utf16_lossy(&units)
            });
            (
                formats,
                text,
                clipboard_files(),
                clipboard_bytes(CF_DIB.0 as u32),
            )
        };
        let (text_value, total_chars, truncated) = match &text {
            Some(text) => {
                let total = text.chars().count();
                (
                    Some(text.chars().take(max_chars).collect::<String>()),
                    Some(total),
                    total > max_chars,
                )
            }
            None => (None, None, false),
        };
        let image = match dib {
            Some(dib) => {
                let (width, height, rgba) = dib_to_rgba(&dib)?;
                match image_output {
                    Some(path) => {
                        let mut saved = write_png(path, width, height, rgba, overwrite)?;
                        saved["saved"] = json!(true);
                        saved
                    }
                    None => json!({"width":width,"height":height,"saved":false}),
                }
            }
            None if image_output.is_some() => {
                return Err("The clipboard does not contain an image.".into());
            }
            None => Value::Null,
        };
        Ok(json!({
            "sequenceNumber":unsafe { GetClipboardSequenceNumber() },"formats":formats,
            "text":text_value,"textChars":total_chars,"textTruncated":truncated,
            "files":files,"image":image
        }))
    }

    fn clipboard_set(args: &Value) -> Result<Value, String> {
        let text = required_str(args, "text")?;
        if text.len() > 16 * 1024 * 1024 {
            return Err("'text' may be at most 16 MiB.".into());
        }
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = wide.len() * 2;
        unsafe {
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes)
                .map_err(|e| format!("Cannot allocate clipboard memory: {e}"))?;
            let pointer = GlobalLock(memory) as *mut u16;
            if pointer.is_null() {
                let _ = GlobalFree(Some(memory));
                return Err("Cannot lock clipboard memory.".into());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), pointer, wide.len());
            let _ = GlobalUnlock(memory);
            let clipboard = match Clipboard::open() {
                Ok(clipboard) => clipboard,
                Err(e) => {
                    let _ = GlobalFree(Some(memory));
                    return Err(e);
                }
            };
            let stored = EmptyClipboard()
                .and_then(|()| SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(memory.0))));
            drop(clipboard);
            if let Err(e) = stored {
                // The system owns the block only after SetClipboardData succeeds.
                let _ = GlobalFree(Some(memory));
                return Err(format!("Cannot set clipboard text: {e}"));
            }
        }
        Ok(json!({
            "chars":text.chars().count(),"bytes":bytes,
            "sequenceNumber":unsafe { GetClipboardSequenceNumber() }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::dib_to_rgba;

    fn dib(height: i32, bits: u16, compression: u32, pixels: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&40u32.to_le_bytes());
        data.extend_from_slice(&2i32.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&bits.to_le_bytes());
        data.extend_from_slice(&compression.to_le_bytes());
        data.extend_from_slice(&[0; 20]);
        if compression == 3 {
            for mask in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF] {
                data.extend_from_slice(&mask.to_le_bytes());
            }
        }
        data.extend_from_slice(pixels);
        data
    }

    #[test]
    fn decodes_bottom_up_24_bit_rows_with_padding() {
        // Rows are 6 bytes padded to 8; the first stored row is the bottom of the image.
        let pixels = [
            0, 0, 255, 0, 255, 0, 0, 0, // bottom: red, green
            255, 0, 0, 255, 255, 255, 0, 0, // top: blue, white
        ];
        let (w, h, rgba) = dib_to_rgba(&dib(2, 24, 0, &pixels)).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(
            rgba,
            [
                0, 0, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255
            ]
        );
    }

    #[test]
    fn decodes_top_down_bitfields_and_keeps_real_alpha() {
        let pixels = [1, 2, 3, 128, 4, 5, 6, 0];
        let (_, h, rgba) = dib_to_rgba(&dib(-1, 32, 3, &pixels)).unwrap();
        assert_eq!(h, 1);
        assert_eq!(rgba, [3, 2, 1, 128, 6, 5, 4, 0]);
    }

    #[test]
    fn zero_alpha_is_treated_as_opaque() {
        let (_, _, rgba) = dib_to_rgba(&dib(1, 32, 0, &[1, 2, 3, 0, 4, 5, 6, 0])).unwrap();
        assert_eq!(rgba, [3, 2, 1, 255, 6, 5, 4, 255]);
    }

    #[test]
    fn rejects_truncated_and_palettized_bitmaps() {
        assert!(dib_to_rgba(&dib(2, 24, 0, &[0; 8])).is_err());
        assert!(dib_to_rgba(&dib(1, 8, 0, &[0; 4])).is_err());
    }
}
