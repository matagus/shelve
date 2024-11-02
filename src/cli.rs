use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    pub filenames: Vec<String>,

    /// Column number to group by
    #[arg(short, long, default_value = "1")]
    pub column_number: Option<u8>,
}
