use std::error::Error;
use std::io::Read;
use std::path::PathBuf;

struct StoneFetch {
    url: String,
    output: PathBuf,
    agent: String,
}

impl StoneFetch {
    const USAGE: &'static str = "usage: stone-fetch <url> <out-file> [user-agent]";
    const AGENT: &'static str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

    fn from_args() -> Result<Self, Box<dyn Error>> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.as_slice() {
            [url, output] => Ok(Self { url: url.clone(), output: PathBuf::from(output), agent: Self::AGENT.to_owned() }),
            [url, output, agent] => Ok(Self { url: url.clone(), output: PathBuf::from(output), agent: agent.clone() }),
            _ => Err(Self::USAGE.into()),
        }
    }

    fn run(&self) -> Result<(), Box<dyn Error>> {
        let response = ureq::get(&self.url).set("User-Agent", &self.agent).call()?;
        let kind = response.content_type().to_owned();
        let mut bytes = Vec::new();
        response.into_reader().read_to_end(&mut bytes)?;
        if let Some(parent) = self.output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.output, &bytes)?;
        println!("wrote {} ({} bytes, {kind})", self.output.display(), bytes.len());
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    StoneFetch::from_args()?.run()
}
