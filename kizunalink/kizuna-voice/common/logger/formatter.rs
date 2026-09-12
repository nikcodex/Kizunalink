// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use core::fmt::{self as core_fmt};

use tracing::{Event, Level, Subscriber};
use tracing_subscriber::{
    fmt::{
        self, FmtContext,
        format::{FormatEvent, FormatFields},
    },
    registry::LookupSpan,
};

use crate::common::utils::{
    BOLD, COLOR_DEBUG, COLOR_ERROR, COLOR_INFO, COLOR_TRACE, COLOR_WARN, DIM, RESET,
    memory_usage_report, CYAN,
};

pub struct CustomFormatter {
    use_ansi: bool,
}

impl CustomFormatter {
    pub fn new(use_ansi: bool) -> Self {
        Self { use_ansi }
    }

    fn get_timestamp(&self) -> String {
        let format = time::macros::format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"
        );
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        now.format(&format).unwrap_or_else(|_| "Unknown Time".to_string())
    }

    fn get_level_color(&self, level: &Level) -> &'static str {
        if !self.use_ansi {
            return "";
        }
        match *level {
            Level::ERROR => COLOR_ERROR,
            Level::WARN => COLOR_WARN,
            Level::INFO => COLOR_INFO,
            Level::DEBUG => COLOR_DEBUG,
            Level::TRACE => COLOR_TRACE,
        }
    }
}

impl<S, N> FormatEvent<S, N> for CustomFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: fmt::format::Writer<'_>,
        event: &Event<'_>,
    ) -> core_fmt::Result {
        let (reset, dim, bold) = if self.use_ansi {
            (RESET, DIM, BOLD)
        } else {
            ("", "", "")
        };

        let timestamp = self.get_timestamp();
        let metadata = event.metadata();
        let level = metadata.level();
        let level_color = self.get_level_color(level);
        let ram = memory_usage_report();

        // ╭─[ Timestamp ]─[ RAM ]─[ LEVEL ]
        write!(
            writer,
            "{dim}╭─{reset} {dim}[{reset}{timestamp}{dim}]{reset} {dim}─{reset} {dim}[{reset}{CYAN}{ram}{reset}{dim}]{reset} {dim}─{reset} {dim}[{reset}{level_color}{bold}{level: <5}{reset}{dim}]{reset}\n"
        )?;

        let target = metadata.target();
        let line = metadata
            .line()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "??".to_string());
        
        // │  target: kizunalink::media::sources::...:79
        write!(writer, "{dim}│{reset}  {dim}at:{reset} {target}:{line}\n")?;

        // ╰─> Message
        write!(writer, "{dim}╰─>{reset} ")?;
        ctx.format_fields(writer.by_ref(), event)?;
        
        write!(writer, "{reset}\n")?;

        Ok(())
    }
}
