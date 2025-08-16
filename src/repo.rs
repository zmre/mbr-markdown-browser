use std::{
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use papaya::{HashMap, HashSet};
use rayon::prelude::*;
use walkdir::{DirEntry, WalkDir};

use crate::Config;

#[derive(Clone)]
pub struct Repo {
    root_dir: PathBuf,
    static_folder: String,
    markdown_extensions: Vec<String>,
    index_file: String,
    pub scanned_folders: HashSet<PathBuf>,
    pub queued_folders: HashMap<PathBuf, PathBuf>,
    pub markdown_files: HashMap<PathBuf, MarkdownInfo>,
    pub other_files: HashMap<PathBuf, OtherFileInfo>,
}

#[derive(Clone)]
struct MarkdownInfo {
    raw_path: PathBuf,
    url_path: String,
    created: u64,
    modified: u64,
    pub metadata: Option<crate::markdown::SimpleMetadata>,
}

#[derive(Clone)]
struct OtherFileInfo {
    raw_path: PathBuf,
    url_path: String,
    metadata: StaticFileMetadata,
}

#[derive(Clone, Default)]
struct StaticFileMetadata {
    path: PathBuf,
    created: Option<u64>,
    modified: Option<u64>,
    file_size_bytes: Option<u64>,
    kind: StaticFileKind,
}

#[derive(Clone, Default)]
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

    pub async fn from<P: Into<std::path::PathBuf>>(file: P) -> Self {
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
            c.index_file.clone(),
        )
    }

    pub fn init<S: Into<String>, P: Into<std::path::PathBuf>>(
        root_dir: P,
        static_folder: S,
        markdown_extensions: &[String],
        index_file: S,
    ) -> Self {
        Self {
            root_dir: root_dir.into(),
            static_folder: static_folder.into(),
            markdown_extensions: markdown_extensions.to_vec(),
            index_file: index_file.into(),
            scanned_folders: HashSet::new(),
            queued_folders: HashMap::new(),
            markdown_files: HashMap::new(),
            other_files: HashMap::new(),
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

        self.scanned_folders.pin().insert(start_folder.clone());
        let dir_walker = WalkDir::new(start_folder).follow_links(true).into_iter();
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
            let file_name = path.file_name().map(|x| x.to_str().unwrap()).unwrap_or("");
            let extension = path.extension().map(|x| x.to_str().unwrap()).unwrap_or("");

            if file_name.starts_with('.') {
                continue;
            } else if path.is_dir() {
                self.queued_folders
                    .pin()
                    .insert(path.to_path_buf(), relative_folder_path_ref.join(file_name));
            } else if self
                .markdown_extensions
                .iter()
                .find(|x| x.as_str() == extension)
                .is_some()
            {
                if let Ok((filesize, created, modified)) = file_details_from_path(path) {
                    let url = PathBuf::from("/")
                        .join(relative_folder_path_ref)
                        .to_string_lossy()
                        .to_string();

                    let mdfile = MarkdownInfo {
                        raw_path: path.to_path_buf(),
                        url_path: url,
                        created,
                        modified,
                        metadata: None,
                    };
                    markdown.insert(path.to_path_buf(), mdfile);
                } else {
                    // we're going to ignore errors, but if we can't get basic file info, we aren't adding it. just warn
                    eprintln!("Couldn't process markdown file at {:?}", path);
                }
            } else {
                let url = PathBuf::from("/")
                    .join(relative_folder_path_ref)
                    .to_string_lossy()
                    .replace(("/".to_string() + self.static_folder.as_str()).as_str(), "");

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
                    new_details.metadata = metadata;
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

        // TODO: rayon recurse into each found subfolder; note: would love to just return, but think I need to block on the recursion :(
        //       other option would be to have a self.queue_folders to add to so I can return and those can be processed by scan_all
        //       so that this function only ever scans one dir and can be maximally fast; could then have a scan_folder_recurse()
        //       function that processes subdirs as an option
        Ok(())
    }

    pub async fn scan_all(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.scan_folder(&PathBuf::from("."))?; // the . is relative to the root_dir, so this scans the root dir
        while !self.queued_folders.is_empty() {
            // TODO: make sure this doesn't deadlock
            let vec_folders: Vec<_> = self
                .queued_folders
                .pin()
                .iter()
                .map(|(_, path)| path.clone())
                .collect();
            self.queued_folders.pin().clear();
            assert!(self.queued_folders.is_empty());
            vec_folders.into_par_iter().for_each(|rel_path| {
                self.scan_folder(&rel_path.as_path()).unwrap_or_else(|e| {
                    eprintln!("Failed to scan folder {:?} with error {e}", &rel_path)
                }) // ignores errors
            });
        }
        Ok(())
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
