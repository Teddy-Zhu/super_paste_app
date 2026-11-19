use crate::common::Result;
#[cfg(feature = "image")]
use crate::common::{RustImage, RustImageData};
use crate::{Clipboard, ClipboardContent, ClipboardHandler, ClipboardWatcher, ContentFormat};
use objc2::rc::Retained;
use objc2::AllocAnyThread;
use objc2::ClassType;
use objc2::{rc::autoreleasepool, runtime::ProtocolObject};
use objc2_app_kit::{
	NSImage, NSPasteboard, NSPasteboardItem, NSPasteboardType, NSPasteboardTypeFileURL,
	NSPasteboardTypeHTML, NSPasteboardTypePNG, NSPasteboardTypeRTF, NSPasteboardTypeString,
	NSPasteboardTypeTIFF, NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSData, NSString, NSURL};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use std::vec;

pub struct ClipboardContext {
	pasteboard: Retained<NSPasteboard>,
}

pub struct ClipboardWatcherContext<T: ClipboardHandler> {
	pasteboard: Retained<NSPasteboard>,
	handlers: Vec<T>,
	stop_signal: Sender<()>,
	stop_receiver: Receiver<()>,
	running: bool,
	interval: Duration,
}

unsafe impl<T: ClipboardHandler> Send for ClipboardWatcherContext<T> {}

/// Default polling interval. macOS exposes no clipboard-change event, so the
/// watcher polls `NSPasteboard.changeCount`; this is the wait between polls.
const DEFAULT_WATCH_INTERVAL: Duration = Duration::from_millis(500);

impl<T: ClipboardHandler> ClipboardWatcherContext<T> {
	pub fn new() -> Result<Self> {
		Self::new_with_interval(DEFAULT_WATCH_INTERVAL)
	}

	/// Creates a watcher that polls the pasteboard every `interval`.
	///
	/// macOS has no clipboard-change notification API, so changes are detected by
	/// polling `changeCount`. A smaller `interval` lowers detection latency at the
	/// cost of more frequent wake-ups; `new` uses [`DEFAULT_WATCH_INTERVAL`].
	pub fn new_with_interval(interval: Duration) -> Result<Self> {
		let ns_pasteboard = NSPasteboard::generalPasteboard();
		let (tx, rx) = mpsc::channel();
		Ok(ClipboardWatcherContext {
			pasteboard: ns_pasteboard,
			handlers: Vec::new(),
			stop_signal: tx,
			stop_receiver: rx,
			running: false,
			interval,
		})
	}
}

impl<T: ClipboardHandler> ClipboardWatcher<T> for ClipboardWatcherContext<T> {
	fn add_handler(&mut self, handler: T) -> &mut Self {
		self.handlers.push(handler);
		self
	}

	fn start_watch(&mut self) {
		if self.running {
			println!("already start watch!");
			return;
		}
		if self.handlers.is_empty() {
			println!("no handler, no need to start watch!");
			return;
		}
		self.running = true;
		let mut last_change_count = self.pasteboard.changeCount();
		loop {
			// if receive stop signal, break loop
			if self.stop_receiver.recv_timeout(self.interval).is_ok() {
				break;
			}
			let change_count = self.pasteboard.changeCount();
			if last_change_count == 0 {
				last_change_count = change_count;
			} else if change_count != last_change_count {
				self.handlers
					.iter_mut()
					.for_each(|handler| handler.on_clipboard_change());
				last_change_count = change_count;
			}
		}
		self.running = false;
	}

	fn get_shutdown_channel(&self) -> WatcherShutdown {
		WatcherShutdown {
			stop_signal: self.stop_signal.clone(),
		}
	}
}

impl ClipboardContext {
	pub fn new() -> Result<ClipboardContext> {
		let ns_pasteboard = NSPasteboard::generalPasteboard();
		let clipboard_ctx = ClipboardContext {
			pasteboard: ns_pasteboard,
		};
		Ok(clipboard_ctx)
	}

	fn plain(&self, r#type: &NSPasteboardType) -> Result<String> {
		autoreleasepool(|_| {
			let contents = self
				.pasteboard
				.pasteboardItems()
				.ok_or("NSPasteboard#pasteboardItems errored")?;
			for item in contents {
				if let Some(string) = item.stringForType(r#type) {
					return Ok(string.to_string());
				}
			}
			Err("No string found".into())
		})
	}

	fn file_objects(
		files: &[String],
	) -> Result<Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>>> {
		if files.is_empty() {
			return Err("file list is empty".into());
		}
		files
			.iter()
			.map(|file| {
				let path = file.strip_prefix("file://").unwrap_or(file);
				if !std::path::Path::new(path).try_exists()? {
					return Err("clipboard file no longer exists".into());
				}
				let url = NSURL::fileURLWithPath(&NSString::from_str(path));
				Ok(ProtocolObject::from_retained(url))
			})
			.collect()
	}

	fn write_to_clipboard(&self, data: &[ClipboardContent]) -> Result<()> {
		autoreleasepool(|_| {
			let item = NSPasteboardItem::new();
			let mut objects = Vec::new();
			let mut has_content = false;

			for content in data {
				let written = match content {
					ClipboardContent::Text(text) => item
						.setString_forType(&NSString::from_str(text), unsafe {
							NSPasteboardTypeString
						}),
					ClipboardContent::Rtf(rtf) => item
						.setData_forType(&NSData::with_bytes(rtf.as_bytes()), unsafe {
							NSPasteboardTypeRTF
						}),
					ClipboardContent::Html(html) => item
						.setString_forType(&NSString::from_str(html), unsafe {
							NSPasteboardTypeHTML
						}),
					#[cfg(feature = "image")]
					ClipboardContent::Image(image) => {
						let png = image.to_png()?;
						item.setData_forType(&NSData::with_bytes(png.get_bytes()), unsafe {
							NSPasteboardTypePNG
						})
					}
					ClipboardContent::Files(files) => {
						objects.extend(Self::file_objects(files)?);
						continue;
					}
					ClipboardContent::Other(format, bytes) => item
						.setData_forType(&NSData::with_bytes(bytes), &NSString::from_str(format)),
				};
				if !written {
					return Err("could not prepare pasteboard item".into());
				}
				has_content = true;
			}
			if has_content {
				objects.push(ProtocolObject::from_retained(item));
			}
			let objects = NSArray::from_retained_slice(&objects);
			self.pasteboard.clearContents();
			if !self.pasteboard.writeObjects(&objects) {
				return Err("writeObjects failed".into());
			}
			Ok(())
		})
	}
}

unsafe impl Send for ClipboardContext {}

unsafe impl Sync for ClipboardContext {}

impl Clipboard for ClipboardContext {
	fn available_formats(&self) -> Result<Vec<String>> {
		let types = self
			.pasteboard
			.types()
			.ok_or("NSPasteboard#types errored")?;
		let res = types.iter().map(|t| t.to_string()).collect();
		Ok(res)
	}

	fn has(&self, format: ContentFormat) -> bool {
		match format {
			ContentFormat::Text => {
				let types = NSArray::arrayWithObject(unsafe { NSPasteboardTypeString });
				// https://developer.apple.com/documentation/appkit/nspasteboard/1526078-availabletypefromarray?language=objc
				// The first pasteboard type in types that is available on the pasteboard, or nil if the receiver does not contain any of the types in types.
				// self.clipboard.availableTypeFromArray(types)
				self.pasteboard.availableTypeFromArray(&types).is_some()
			}
			ContentFormat::Rtf => {
				let types = NSArray::arrayWithObject(unsafe { NSPasteboardTypeRTF });
				self.pasteboard.availableTypeFromArray(&types).is_some()
			}
			ContentFormat::Html => {
				// Currently only judge whether there is a public.html format
				let types = NSArray::arrayWithObject(unsafe { NSPasteboardTypeHTML });
				self.pasteboard.availableTypeFromArray(&types).is_some()
			}
			#[cfg(feature = "image")]
			ContentFormat::Image => {
				// Currently only judge whether there is a png format
				let types = NSArray::from_retained_slice(&[
					unsafe { NSPasteboardTypePNG }.to_owned(),
					unsafe { NSPasteboardTypeTIFF }.to_owned(),
				]);
				self.pasteboard.availableTypeFromArray(&types).is_some()
			}
			ContentFormat::Files => {
				let types = NSArray::arrayWithObject(unsafe { NSPasteboardTypeFileURL });
				self.pasteboard.availableTypeFromArray(&types).is_some()
			}
			ContentFormat::Other(format) => {
				let types = NSArray::from_retained_slice(&[NSString::from_str(&format)]);
				self.pasteboard.availableTypeFromArray(&types).is_some()
			}
		}
	}

	fn clear(&self) -> Result<()> {
		self.pasteboard.clearContents();
		Ok(())
	}

	fn get_buffer(&self, format: &str) -> Result<Vec<u8>> {
		if let Some(data) = self.pasteboard.dataForType(&NSString::from_str(format)) {
			return Ok(data.to_vec());
		}
		Err("no data".into())
	}

	fn get_text(&self) -> Result<String> {
		self.plain(unsafe { NSPasteboardTypeString })
	}

	fn get_rich_text(&self) -> Result<String> {
		self.plain(unsafe { NSPasteboardTypeRTF })
	}

	fn get_html(&self) -> Result<String> {
		self.plain(unsafe { NSPasteboardTypeHTML })
	}

	#[cfg(feature = "image")]
	fn get_image(&self) -> Result<RustImageData> {
		autoreleasepool(|_| {
			let png_data = self.pasteboard.dataForType(unsafe { NSPasteboardTypePNG });
			if let Some(data) = png_data {
				return RustImageData::from_bytes(&data.to_vec());
			};
			// if no png data, read NSImage;
			let ns_image = NSImage::initWithPasteboard(NSImage::alloc(), &self.pasteboard);
			if let Some(image) = ns_image {
				let tiff_data = image.TIFFRepresentation();
				if let Some(data) = tiff_data {
					return RustImageData::from_bytes(&data.to_vec());
				}
			};
			Err("no image data".into())
		})
	}

	fn get_files(&self) -> Result<Vec<String>> {
		autoreleasepool(|_| {
			let mut res = vec![];
			// 使用 readObjectsForClasses 读取 NSURL（文件 URL）
			// 相当于 Objective-C: [pasteboard readObjectsForClasses:@[[NSURL class]] options:nil]
			let classes = NSArray::arrayWithObject(NSURL::class());
			let objects = unsafe {
				self.pasteboard
					.readObjectsForClasses_options(&classes, None)
			};
			if let Some(objects) = objects {
				for any_obj in objects {
					// 尝试将返回对象视为 NSURL
					if let Ok(url) = any_obj.downcast::<NSURL>() {
						// 只接受文件 URL
						if url.isFileURL() {
							// 优先使用 path（去掉 file:// 前缀后的本地路径）
							if let Some(path) = url.path() {
								res.push(path.to_string());
							} else {
								// 兜底使用 absoluteString，再去掉前缀
								let abs = if let Some(s) = url.absoluteString() {
									s.to_string()
								} else {
									String::new()
								};
								let file_path = if abs.starts_with("file://") {
									abs.strip_prefix("file://").unwrap_or(&abs).to_string()
								} else {
									abs
								};
								res.push(file_path);
							}
						}
					}
				}
			}
			if res.is_empty() {
				return Err("no files".into());
			}
			Ok(res)
		})
	}

	fn get(&self, formats: &[ContentFormat]) -> Result<Vec<ClipboardContent>> {
		autoreleasepool(|_| {
			let contents = self
				.pasteboard
				.pasteboardItems()
				.ok_or("NSPasteboard#pasteboardItems errored")?;
			let mut results = Vec::new();
			for format in formats {
				for item in contents.iter() {
					match format {
						ContentFormat::Text => {
							if let Some(string) =
								item.stringForType(unsafe { NSPasteboardTypeString })
							{
								results.push(ClipboardContent::Text(string.to_string()));
								break;
							}
						}
						ContentFormat::Rtf => {
							if let Some(string) = item.stringForType(unsafe { NSPasteboardTypeRTF })
							{
								results.push(ClipboardContent::Rtf(string.to_string()));
								break;
							}
						}
						ContentFormat::Html => {
							if let Some(string) =
								item.stringForType(unsafe { NSPasteboardTypeHTML })
							{
								results.push(ClipboardContent::Html(string.to_string()));
								break;
							}
						}
						#[cfg(feature = "image")]
						ContentFormat::Image => {
							if let Ok(image) = self.get_image() {
								results.push(ClipboardContent::Image(image));
								break;
							}
						}
						ContentFormat::Files => {
							if let Ok(files) = self.get_files() {
								results.push(ClipboardContent::Files(files));
								break;
							}
						}
						ContentFormat::Other(format_name) => {
							if let Some(data) = item.dataForType(&NSString::from_str(format_name)) {
								results.push(ClipboardContent::Other(
									format_name.to_string(),
									data.to_vec(),
								));
								break;
							}
						}
					}
				}
			}
			Ok(results)
		})
	}

	fn set_buffer(&self, format: &str, buffer: Vec<u8>) -> Result<()> {
		self.write_to_clipboard(&[ClipboardContent::Other(format.to_owned(), buffer)])
	}

	fn set_text(&self, text: String) -> Result<()> {
		self.write_to_clipboard(&[ClipboardContent::Text(text)])
	}

	fn set_rich_text(&self, text: String) -> Result<()> {
		self.write_to_clipboard(&[ClipboardContent::Rtf(text)])
	}

	fn set_html(&self, html: String) -> Result<()> {
		self.write_to_clipboard(&[ClipboardContent::Html(html)])
	}

	#[cfg(feature = "image")]
	fn set_image(&self, image: RustImageData) -> Result<()> {
		self.write_to_clipboard(&[ClipboardContent::Image(image)])
	}

	fn set_files(&self, files: Vec<String>) -> Result<()> {
		self.write_to_clipboard(&[ClipboardContent::Files(files)])
	}

	fn set(&self, contents: Vec<ClipboardContent>) -> Result<()> {
		if contents.is_empty() {
			return Err(
				"contents is empty, if you want to clear clipboard, please use clear method".into(),
			);
		}
		self.write_to_clipboard(&contents)
	}
}

pub struct WatcherShutdown {
	stop_signal: Sender<()>,
}

impl Drop for WatcherShutdown {
	fn drop(&mut self) {
		let _ = self.stop_signal.send(());
	}
}
