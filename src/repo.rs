use std::{
    ops::Deref,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use papaya::{HashMap, HashSet};
use rayon::prelude::*;
use serde::{
    ser::{SerializeMap, SerializeSeq},
    Serialize, Serializer,
};
use walkdir::{DirEntry, WalkDir};

use crate::Config;

#[derive(Clone, Serialize)]
pub struct Repo {
    #[serde(skip)]
    root_dir: PathBuf,
    #[serde(skip)]
    static_folder: String,
    #[serde(skip)]
    markdown_extensions: Vec<String>,
    #[serde(skip)]
    index_file: String,
    #[serde(skip)]
    ignore_dirs: Vec<String>,
    #[serde(skip)]
    ignore_globs: Vec<String>,
    #[serde(skip)]
    pub scanned_folders: HashSet<PathBuf>,
    #[serde(skip)]
    pub queued_folders: HashMap<PathBuf, PathBuf>,
    pub markdown_files: MarkdownFiles,
    pub other_files: OtherFiles,
}

#[derive(Clone)]
pub struct MarkdownFiles(HashMap<PathBuf, MarkdownInfo>);
impl Deref for MarkdownFiles {
    type Target = HashMap<PathBuf, MarkdownInfo>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Serialize for MarkdownFiles {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_seq(Some(self.len()))?;
        for (_, v) in self.pin().iter() {
            s.serialize_element(v)?;
        }
        s.end()
    }
}

#[derive(Clone)]
pub struct OtherFiles(HashMap<PathBuf, OtherFileInfo>);
impl Deref for OtherFiles {
    type Target = HashMap<PathBuf, OtherFileInfo>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Serialize for OtherFiles {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_seq(Some(self.len()))?;
        for (_, v) in self.pin().iter() {
            s.serialize_element(v)?;
        }
        s.end()
    }
}

#[derive(Clone, Serialize)]
pub struct MarkdownInfo {
    raw_path: PathBuf,
    url_path: String,
    created: u64,
    modified: u64,
    pub frontmatter: Option<crate::markdown::SimpleMetadata>,
}

#[derive(Clone, Serialize)]
pub struct OtherFileInfo {
    raw_path: PathBuf,
    url_path: String,
    metadata: StaticFileMetadata,
}

#[derive(Clone, Default, Serialize)]
pub struct StaticFileMetadata {
    path: PathBuf,
    created: Option<u64>,
    modified: Option<u64>,
    file_size_bytes: Option<u64>,
    kind: StaticFileKind,
}

#[derive(Clone, Default, Serialize)]
enum StaticFileKind {
    Pdf {
        description: Option<String>,
        title: Option<String>,
        author: Option<String>,
        subject: Option<String>,
        num_pages: Option<usize>,
    },
    Image {
        width: Option<u32>,
        height: Option<u32>,
    },
    Video {
        width: Option<u32>,
        height: Option<u32>,
        duration: Option<String>,
        title: Option<String>,
    },
    Audio {
        duration: Option<String>,
        title: Option<String>,
    },
    Text,
    #[default]
    Other,
}

/* impl Serialize for Repo {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("Site", 2)?;
        s.serialize_field("markdown", &self.markdown_files)?;
        s.serialize_field("other", &self.other_files)?;
        s.end()
    }
}

impl Serialize for papaya::HashMap<PathBuf, MarkdownInfo> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.x.len()))?;
        for (k, v) in &self.x {
            map.serialize_entry(&k.to_string(), &v)?;
        }
        map.end()
    }
} */

impl StaticFileMetadata {
    pub fn empty<P: Into<std::path::PathBuf>>(file: P) -> Self {
        let file: PathBuf = file.into();
        // We'll silently ignore errors and always return something
        match file
            .extension()
            .map(|x| x.to_ascii_lowercase().to_string_lossy().to_string())
            .as_deref()
        {
            Some("pdf") => Self {
                path: file,
                kind: StaticFileKind::Pdf {
                    description: None,
                    title: None,
                    author: None,
                    subject: None,
                    num_pages: None,
                },
                ..Default::default()
            },
            Some("jpg") | Some("jpeg") | Some("png") | Some("webp") | Some("gif") | Some("bmp")
            | Some("tif") | Some("tiff") => Self {
                path: file,
                kind: StaticFileKind::Image {
                    width: None,
                    height: None,
                },
                ..Default::default()
            },
            Some("aiff") | Some("mp3") | Some("aac") | Some("m4a") | Some("ogg") | Some("oga")
            | Some("opus") | Some("wma") | Some("flac") | Some("wav") | Some("aif") | Some("") => {
                Self {
                    path: file,
                    kind: StaticFileKind::Audio {
                        duration: None,
                        title: None,
                    },
                    ..Default::default()
                }
            }
            Some("mp4") | Some("m4v") | Some("mov") | Some("webm") | Some("flv") | Some("mpg")
            | Some("mpeg") | Some("avi") | Some("3gp") | Some("wmv") => Self {
                path: file,
                kind: StaticFileKind::Video {
                    width: None,
                    height: None,
                    duration: None,
                    title: None,
                },
                ..Default::default()
            },
            Some("txt") | Some("css") | Some("vtt") | Some("toml") | Some("json") | Some("js")
            | Some("ts") => Self {
                path: file,
                kind: StaticFileKind::Text,
                ..Default::default()
            },
            _ => Self {
                path: file,
                kind: StaticFileKind::Other,
                ..Default::default()
            },
        }
    }

    pub fn populate(self) -> Self {
        let mut me = self;
        let (filesize, created, modified) = match file_details_from_path(&me.path).ok() {
            Some((fs, c, m)) => (Some(fs), Some(c), Some(m)),
            _ => (None, None, None),
        };
        me.file_size_bytes = filesize;
        me.created = created;
        me.modified = modified;
        me.kind = match me.kind {
            /* StaticFileKind::Pdf {
                            ..me
                        }, // TODO: get PDF metadata using https://docs.rs/pdf-extract/latest/pdf_extract/ -- but see if there's a way to just process some of the file
            // */
            StaticFileKind::Image { .. } => {
                let metadata =
                    metadata::media_file::MediaFileMetadata::new(&me.path.as_path()).ok();

                StaticFileKind::Image {
                    width: metadata.as_ref().map(|m| m.width).flatten(),
                    height: metadata.as_ref().map(|m| m.height).flatten(),
                }
            }
            StaticFileKind::Audio { .. } => {
                let metadata =
                    metadata::media_file::MediaFileMetadata::new(&me.path.as_path()).ok();
                StaticFileKind::Audio {
                    duration: metadata.as_ref().map(|m| m.duration.clone()).flatten(),
                    title: metadata.as_ref().map(|m| m.title.clone()).flatten(),
                }
            }
            StaticFileKind::Video { .. } => {
                let metadata =
                    metadata::media_file::MediaFileMetadata::new(&me.path.as_path()).ok();

                StaticFileKind::Video {
                    width: metadata.as_ref().map(|m| m.width).flatten(),
                    height: metadata.as_ref().map(|m| m.height).flatten(),
                    duration: metadata.as_ref().map(|m| m.duration.clone()).flatten(),
                    title: metadata.as_ref().map(|m| m.title.clone()).flatten(),
                }
            }
            _ => me.kind,
        };
        me
    }

    pub fn from<P: Into<std::path::PathBuf>>(file: P) -> Self {
        let empty = Self::empty(file);
        empty.populate()
    }
}

impl Repo {
    pub fn init_from_config(c: &Config) -> Self {
        Self::init(
            c.root_dir.clone(),
            c.static_folder.clone(),
            &c.markdown_extensions[..],
            &c.ignore_dirs[..],
            &c.ignore_globs[..],
            c.index_file.clone(),
        )
    }

    pub fn init<S: Into<String>, P: Into<std::path::PathBuf>>(
        root_dir: P,
        static_folder: S,
        markdown_extensions: &[String],
        ignore_dirs: &[String],
        ignore_globs: &[String],
        index_file: S,
    ) -> Self {
        Self {
            root_dir: root_dir.into(),
            static_folder: static_folder.into(),
            markdown_extensions: markdown_extensions.to_vec(),
            ignore_dirs: ignore_dirs.to_vec(),
            ignore_globs: ignore_globs.to_vec(),
            index_file: index_file.into(),
            scanned_folders: HashSet::new(),
            queued_folders: HashMap::new(),
            markdown_files: MarkdownFiles(HashMap::new()),
            other_files: OtherFiles(HashMap::new()),
        }
    }

    pub fn scan_folder<P: AsRef<Path>>(
        &self,
        relative_folder_path: &P,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let relative_folder_path_ref = relative_folder_path.as_ref();
        let start_folder = self
            .root_dir
            .join(relative_folder_path_ref)
            .canonicalize()?;
        if self.scanned_folders.pin().contains(&start_folder) {
            return Ok(());
        }
        println!("relative_folder: {:?}", relative_folder_path_ref);

        self.scanned_folders.pin().insert(start_folder.clone());
        let dir_walker = WalkDir::new(start_folder.clone())
            .follow_links(true)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_entry(|e| {
                let path = e.path();
                let file_name = path.file_name().map(|x| x.to_str().unwrap()).unwrap_or("");
                // false skips
                !(file_name.starts_with('.')
                    || (path.is_dir() && self.ignore_dirs.iter().any(|x| x.as_str() == file_name))
                    || self.ignore_globs.iter().any(|pat| {
                        glob::Pattern::new(pat)
                            .map(|pat| pat.matches_path(path))
                            .unwrap_or(false)
                    }))
            });
        let mut markdown = std::collections::HashMap::new();
        let mut other = std::collections::HashMap::new();

        for entry in dir_walker {
            // TODO handle ignores and also ignore paths and files with leading dots
            // no use doing this in parallel since Mac makes you single thread operations on a single folder
            let entry: DirEntry = match entry {
                Err(_) => continue,
                Ok(en) => en,
            };
            let path = entry.path();
            let extension = path.extension().map(|x| x.to_str().unwrap()).unwrap_or("");

            if path.is_dir() {
                let relative_entry =
                    pathdiff::diff_paths(path, &self.root_dir).unwrap_or(path.to_path_buf());
                self.queued_folders
                    .pin()
                    .insert(path.to_path_buf(), relative_entry);
            } else if self
                .markdown_extensions
                .iter()
                .any(|x| x.as_str() == extension)
            {
                if let Ok((_filesize, created, modified)) = file_details_from_path(path) {
                    // let url = PathBuf::from("/")
                    //     .join(relative_folder_path_ref)
                    //     .to_string_lossy()
                    //     .to_string();

                    let mut url = pathdiff::diff_paths(path, &self.root_dir)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or("".to_string());
                    if !url.starts_with('/') {
                        url = "/".to_string() + &url;
                    }
                    if url.ends_with(&self.index_file) {
                        url = url.replace(&self.index_file, "");
                    }
                    if let Some((base, extension)) = url.rsplit_once('.') {
                        if !extension.contains('/') {
                            url = base.to_string() + "/";
                        }
                    }

                    let mdfile = MarkdownInfo {
                        raw_path: path.to_path_buf(),
                        url_path: url,
                        created,
                        modified,
                        frontmatter: None,
                    };
                    markdown.insert(path.to_path_buf(), mdfile);
                } else {
                    // we're going to ignore errors, but if we can't get basic file info, we aren't adding it. just warn
                    eprintln!("Couldn't process markdown file at {:?}", path);
                }
            } else {
                // let url = PathBuf::from("/")
                //     .join(relative_folder_path_ref)
                //     .to_string_lossy()
                //     .replace(("/".to_string() + self.static_folder.as_str()).as_str(), "");
                let mut url = pathdiff::diff_paths(path, &self.root_dir)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or("".to_string())
                    .replace(&self.static_folder, "");
                if !url.starts_with('/') {
                    url = "/".to_string() + &url;
                }

                let other_file = OtherFileInfo {
                    raw_path: path.to_path_buf(),
                    url_path: url,
                    metadata: StaticFileMetadata::empty(path),
                };
                other.insert(path.to_path_buf(), other_file);
            }
        }

        // use rayon to run through all found markdown files and flesh out details (did i use papaya and rayon together properly?)
        // add to the collection of markdown files in self
        markdown
            .into_par_iter()
            .for_each(|(mdfile, mddetails): (PathBuf, MarkdownInfo)| {
                let metadata = crate::markdown::extract_metadata_from_file(&mdfile).ok();
                if metadata.is_some() {
                    let mut new_details = mddetails;
                    new_details.frontmatter = metadata;
                    self.markdown_files.pin().insert(mdfile, new_details);
                } else {
                    self.markdown_files.pin().insert(mdfile, mddetails);
                }
            });

        // use rayon to run through the static files and flesh out details then add to the main collection
        other.into_par_iter().for_each(|(file, other_file)| {
            let mut other_file = other_file;
            other_file.metadata = other_file.metadata.populate();
            self.other_files.pin().insert(file, other_file);
        });

        Ok(())
    }

    pub fn scan_all(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.scan_folder(&PathBuf::from("."))?; // the . is relative to the root_dir, so this scans the root dir

        // Only scan static folder if it exists
        let static_path = self.root_dir.join(&self.static_folder);
        if static_path.is_dir() {
            self.scan_folder(&PathBuf::from(&self.static_folder))?;
        }

        while !self.queued_folders.is_empty() {
            // TODO: make sure this doesn't deadlock
            let vec_folders: Vec<_> = self
                .queued_folders
                .pin()
                .iter()
                .map(|(absolute, relative)| relative.clone())
                .collect();
            self.queued_folders.pin().clear();
            assert!(self.queued_folders.is_empty());
            eprintln!("Parallel batch: {:?}", &vec_folders);
            vec_folders.into_par_iter().for_each(|rel_path| {
                self.scan_folder(&rel_path).unwrap_or_else(|e| {
                    eprintln!("Failed to scan folder {:?} with error {e}", &rel_path)
                }) // ignores errors
            });
        }
        Ok(())
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }
}

/// Returns file_size, created_secs, modified_secs
pub fn file_details_from_path<P: AsRef<Path>>(
    path: P,
) -> Result<(u64, u64, u64), Box<dyn std::error::Error>> {
    let path = path.as_ref();
    let metadata = std::fs::metadata(path)?;

    let file_size = metadata.len();

    // Modified time
    let modified = metadata.modified()?;
    let modified_secs = modified.duration_since(UNIX_EPOCH)?.as_secs();

    // Created time (might not be supported on all platforms)
    let created = metadata.created()?;
    let created_secs = created.duration_since(UNIX_EPOCH)?.as_secs();

    Ok((file_size, created_secs, modified_secs))
}
