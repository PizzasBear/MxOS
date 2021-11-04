use core::fmt::{self, Write};
use lazy_static::lazy_static;
use uart_16550::SerialPort;

lazy_static! {
    /// The serial port.
    static ref SERIAL1: spin::Mutex<SerialPort> = {
        let mut serial_port = unsafe { SerialPort::new(0x3f8) };
        serial_port.init();
        spin::Mutex::new(serial_port)
    };
    pub static ref SERIAL_LOGGER: SerialLogger = SerialLogger {
        serial: &*SERIAL1,
    };
}

pub struct SerialLogger {
    serial: &'static spin::Mutex<SerialPort>,
}

impl SerialLogger {
    pub unsafe fn force_unlock(&self) {
        self.serial.force_unlock();
    }
}

impl log::Log for SerialLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            writeln!(
                &mut self.serial.lock(),
                "{}: {}",
                record.level(),
                record.args()
            )
            .expect("Failed to write to logging serial");
        }
    }
    fn flush(&self) {}
}

pub fn init_logger() {
    log::set_logger(&*SERIAL_LOGGER).expect("Failed to set logger");
    log::set_max_level(log::LevelFilter::Info);
}

pub struct Indent<T: fmt::Debug> {
    indent: u8,
    value: T,
}

struct IndentWriter<'a, 'b> {
    indent: u8,
    f: &'a mut fmt::Formatter<'b>,
}

impl<'a, 'b> Write for IndentWriter<'a, 'b> {
    fn write_str(&mut self, mut s: &str) -> fmt::Result {
        while let Some(newline_pos) = s.find('\n') {
            self.f.write_str(&s[..=newline_pos])?;
            for _ in 0..self.indent {
                self.f.write_str("    ")?;
            }
            s = &s[newline_pos + 1..];
        }
        self.f.write_str(s)?;
        Ok(())
    }
}

impl<T: fmt::Debug> Indent<T> {
    pub fn new(indent: u8, value: T) -> Self {
        Self { indent, value }
    }
}

impl<T: fmt::Debug> fmt::Debug for Indent<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut writer = IndentWriter {
            indent: self.indent,
            f,
        };
        if writer.f.alternate() {
            write!(&mut writer, "{:#?}", &self.value)
        } else {
            write!(&mut writer, "{:?}", &self.value)
        }
    }
}

pub fn _sprint(args: core::fmt::Arguments) {
    SERIAL1
        .lock()
        .write_fmt(args)
        .expect("Printing to serial failed");
}

/// Print to serial port.
#[macro_export]
macro_rules! sprint {
    ($($arg:tt)*) => {{
        $crate::serial::_sprint(format_args!($($arg)*));
    }};
}

/// Print to serial port with newline.
#[macro_export]
macro_rules! sprintln {
    () => {{
        $crate::sprint!("\n");
    }};
    ($fmt:expr) => {{
        $crate::sprint!(concat!($fmt, "\n"));
    }};
    ($fmt:expr, $($arg:tt)*) => {{
        $crate::sprint!(concat!($fmt, "\n"), $($arg)*);
    }};
}
