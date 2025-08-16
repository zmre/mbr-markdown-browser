use serde::{Deserialize, Serialize, Serializer};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IpArray(pub [u8; 4]);

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub root_dir: PathBuf,
    pub ip: IpArray,
    pub port: u16,
    pub static_folder: String,
    pub enable_writes: bool,
    pub markdown_extensions: Vec<String>,
    pub theme: String,
    pub index_file: String,
    // TODO: need to add some sort order stuff to determine how to arrange files -- by filename, title, last modified or whatever
}

impl std::fmt::Display for IpArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c, d] = self.0;
        write!(f, "{a}.{b}.{c}.{d}")
    }
}

impl<'de> Deserialize<'de> for IpArray {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let ip_str = String::deserialize(deserializer)?;
        let ip: IpAddr = ip_str.parse().map_err(serde::de::Error::custom)?;

        match ip {
            IpAddr::V4(v4) => Ok(IpArray(v4.octets())),
            IpAddr::V6(_) => Err(serde::de::Error::custom("IPv6 addresses are not supported")),
        }
    }
}

impl Serialize for IpArray {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let ip = std::net::Ipv4Addr::from(self.0);
        serializer.serialize_str(&ip.to_string())
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            root_dir: std::env::current_dir().unwrap(),
            ip: IpArray([127, 0, 0, 1]),
            port: 5200,
            static_folder: "static".to_string(),
            enable_writes: false,
            markdown_extensions: vec!["md".to_string()],
            theme: "default".to_string(),
            index_file: "index.md".to_string(),
        }
    }
}

impl Config {
    pub fn read(search_config_from: &PathBuf) -> Result<Self, figment::Error> {
        let default_config = Config::default();
        let root_dir = Self::find_root_dir(search_config_from);
        let mut config: Config = Figment::new()
            .merge(Serialized::defaults(default_config))
            .merge(Env::prefixed("MBR_"))
            .merge(Toml::file(root_dir.join(".mbr/config.toml")))
            .extract()?;
        println!("config: {:?}", &config);
        config.root_dir = root_dir;
        Ok(config)
    }

    fn find_root_dir(start_dir: &PathBuf) -> PathBuf {
        Self::search_folder_in_ancestors(start_dir, ".mbr")
            .or(Self::cwd_if_ancestor(start_dir))
            .unwrap_or(start_dir.clone())
    }

    fn cwd_if_ancestor(start_path: &PathBuf) -> Option<PathBuf> {
        let cwd = std::env::current_dir().unwrap();
        let dir = if start_path.is_dir() {
            start_path
        } else {
            start_path.parent()?
        };
        dir.ancestors()
            .find(|candidate| candidate == &cwd)
            .map(|x| x.to_path_buf())
    }

    fn search_folder_in_ancestors<P: AsRef<Path>>(
        start_path: &PathBuf,
        search_folder: P, // the folder I'm looking for (usually .mbr)
    ) -> Option<PathBuf> {
        let search_folder = search_folder.as_ref();
        let dir = if start_path.is_dir() {
            start_path
        } else {
            start_path.parent()?
        };
        // ancestors() yields `dir`, then its parent, then its parent, … until root.
        dir.ancestors()
            .map(|ancestor| ancestor.join(search_folder))
            .find(|candidate| candidate.as_path().is_dir())
            .map(|mbr_dir| mbr_dir.parent().unwrap().to_path_buf())
    }
}
