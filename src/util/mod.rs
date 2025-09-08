use indicatif::{ProgressBar, ProgressFinish, ProgressStyle};

pub fn new_progress_spinner() -> ProgressBar {
    ProgressBar::new_spinner()
        .with_style(
            ProgressStyle::with_template("{spinner} {pos} {elapsed_precise} {wide_msg}")
                .unwrap()
                .tick_strings(&[
                    // Idea from https://github.com/FGRibreau/spinners/blob/master/src/lib.rs
                    "🌑", "🌒", "🌓", "🌔", "🌕", "🌖", "🌗", "🌘",
                ]),
        )
        .with_finish(ProgressFinish::AndLeave)
}

pub fn new_progress_bar() -> ProgressBar {
    ProgressBar::no_length()
        .with_style(
            ProgressStyle::with_template(
                "{bytes} {elapsed_precise} [ {bytes_per_sec} ] [{wide_bar:.cyan/blue}] {percent}% ETA {eta_precise}",
            )
            .unwrap()
            .progress_chars("=> "),
        )
        .with_finish(ProgressFinish::AndLeave)
}
