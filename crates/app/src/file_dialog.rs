//! Save-file dialog that reports which file type the user picked, so exports always get the
//! extension of the selected format.
//!
//! On Linux the XDG desktop portal (`org.freedesktop.portal.FileChooser.SaveFile`) is called
//! directly: it returns the chosen filter in `current_filter`, which `rfd` discards. Elsewhere,
//! or when no portal answers, `rfd` is used and the format is taken from the typed extension.

use std::path::{Path, PathBuf};

/// One entry of the "Save as type" list.
#[derive(Clone, Debug, PartialEq)]
pub struct FileType {
    pub label: String,
    /// Extensions without the dot, the first one is the default.
    pub extensions: Vec<String>,
}

impl FileType {
    pub fn new(label: impl Into<String>, extensions: &[&str]) -> Self {
        Self { label: label.into(), extensions: extensions.iter().map(|e| e.to_string()).collect() }
    }
}

/// What the user chose.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveChoice {
    pub path: PathBuf,
    /// Index into the list of file types, when the dialog reported it.
    pub file_type: Option<usize>,
}

/// Request for a save dialog.
pub struct SaveRequest<'a> {
    pub title: &'a str,
    pub types: &'a [FileType],
    /// File type selected when the dialog opens.
    pub current_type: usize,
    pub folder: Option<PathBuf>,
    /// Initial file name (with extension).
    pub name: String,
}

/// Show the dialog. `None` when cancelled.
pub fn save_file(req: &SaveRequest) -> Option<SaveChoice> {
    #[cfg(target_os = "linux")]
    {
        match portal::save_file(req) {
            Ok(choice) => return choice,
            Err(e) => log::warn!("file chooser portal unavailable ({e}); using the fallback dialog"),
        }
    }
    let mut d = rfd::FileDialog::new().set_title(req.title).set_file_name(&req.name);
    if let Some(f) = &req.folder {
        d = d.set_directory(f);
    }
    // the current type first, so toolkits that default to the first filter show it
    let first = req.current_type.min(req.types.len().saturating_sub(1));
    let mut order: Vec<usize> = vec![first];
    order.extend((0..req.types.len()).filter(|i| *i != first));
    for i in order {
        let t = &req.types[i];
        let exts: Vec<&str> = t.extensions.iter().map(|s| s.as_str()).collect();
        d = d.add_filter(filter_label(t), &exts);
    }
    let path = d.save_file()?;
    let file_type = extension_of(&path).and_then(|e| type_for_extension(req.types, &e));
    Some(SaveChoice { path, file_type })
}

pub fn filter_label(t: &FileType) -> String {
    format!("{} (*.{})", t.label, t.extensions.join(", *."))
}

pub fn extension_of(path: &Path) -> Option<String> {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).filter(|e| !e.is_empty())
}

pub fn type_for_extension(types: &[FileType], ext: &str) -> Option<usize> {
    let ext = ext.to_ascii_lowercase();
    types.iter().position(|t| t.extensions.contains(&ext))
}

/// Give `path` an extension of `t`: kept when it already is one of them, replaced when it is
/// another known file type's extension, appended otherwise (so "part.v2" becomes "part.v2.stl").
pub fn with_type_extension(path: &Path, t: &FileType, all: &[FileType]) -> PathBuf {
    match extension_of(path) {
        Some(e) if t.extensions.contains(&e) => path.to_path_buf(),
        Some(e) if type_for_extension(all, &e).is_some() => path.with_extension(&t.extensions[0]),
        _ => {
            let mut s = path.as_os_str().to_os_string();
            s.push(".");
            s.push(&t.extensions[0]);
            PathBuf::from(s)
        }
    }
}

#[cfg(target_os = "linux")]
mod portal {
    use std::collections::HashMap;

    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedValue, Value};

    use super::{filter_label, SaveChoice, SaveRequest};

    type Filter = (String, Vec<(u32, String)>);

    fn filter_of(t: &super::FileType) -> Filter {
        let globs: Vec<(u32, String)> = t
            .extensions
            .iter()
            .map(|e| {
                // case-insensitive glob: *.[sS][tT][lL]
                let g: String = e.chars().map(|c| if c.is_ascii_alphabetic() { format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase()) } else { c.to_string() }).collect();
                (0u32, format!("*.{g}"))
            })
            .collect();
        (filter_label(t), globs)
    }

    pub fn save_file(req: &SaveRequest) -> Result<Option<SaveChoice>, String> {
        let conn = Connection::session().map_err(|e| e.to_string())?;
        let unique = conn.unique_name().ok_or("no unique bus name")?.to_string();
        let sender = unique.trim_start_matches(':').replace('.', "_");
        let token = format!("triceratops{}", std::process::id());
        let request_path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
        // subscribe to the Response before calling, so it cannot be missed
        let request = Proxy::new(&conn, "org.freedesktop.portal.Desktop", request_path.as_str(), "org.freedesktop.portal.Request").map_err(|e| e.to_string())?;
        let mut responses = request.receive_signal("Response").map_err(|e| e.to_string())?;
        let chooser = Proxy::new(&conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.FileChooser").map_err(|e| e.to_string())?;

        let filters: Vec<Filter> = req.types.iter().map(filter_of).collect();
        let current = filters.get(req.current_type).cloned().unwrap_or_else(|| filters[0].clone());
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("handle_token", Value::from(token.as_str()));
        options.insert("modal", Value::from(true));
        options.insert("current_name", Value::from(req.name.as_str()));
        if let Some(folder) = &req.folder {
            let mut bytes = folder.as_os_str().as_encoded_bytes().to_vec();
            bytes.push(0);
            options.insert("current_folder", Value::from(bytes));
        }
        options.insert("filters", Value::from(filters.clone()));
        options.insert("current_filter", Value::from(current));
        let _handle: zbus::zvariant::OwnedObjectPath = chooser.call("SaveFile", &("", req.title, options)).map_err(|e| e.to_string())?;

        let msg = responses.next().ok_or("portal closed without a response")?;
        let (code, results): (u32, HashMap<String, OwnedValue>) = msg.body().deserialize().map_err(|e| e.to_string())?;
        if code != 0 {
            return Ok(None); // 1 = cancelled, 2 = other end
        }
        let uris: Vec<String> = results.get("uris").and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
        let Some(uri) = uris.first() else { return Ok(None) };
        let path = uri_to_path(uri).ok_or_else(|| format!("not a local file: {uri}"))?;
        let chosen: Option<Filter> = results.get("current_filter").and_then(|v| Filter::try_from(v.try_clone().ok()?).ok());
        let file_type = chosen.and_then(|(label, _)| req.types.iter().position(|t| filter_label(t) == label));
        Ok(Some(SaveChoice { path, file_type }))
    }

    /// file:// URI to a path (percent-decoded).
    pub fn uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
        let rest = uri.strip_prefix("file://")?;
        let rest = rest.find('/').map(|i| &rest[i..])?;
        let mut out: Vec<u8> = Vec::with_capacity(rest.len());
        let b = rest.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() {
                if let Ok(v) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(b[i]);
            i += 1;
        }
        Some(std::path::PathBuf::from(String::from_utf8_lossy(&out).to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types() -> Vec<FileType> {
        vec![FileType::new("3D Models", &["3dm"]), FileType::new("Stereolithography", &["stl"]), FileType::new("IGES", &["igs", "iges"])]
    }

    #[test]
    fn extension_follows_the_chosen_type() {
        let t = types();
        assert_eq!(with_type_extension(Path::new("/a/part.3dm"), &t[1], &t), PathBuf::from("/a/part.stl"));
        assert_eq!(with_type_extension(Path::new("/a/part"), &t[1], &t), PathBuf::from("/a/part.stl"));
        assert_eq!(with_type_extension(Path::new("/a/part.iges"), &t[2], &t), PathBuf::from("/a/part.iges"));
        assert_eq!(with_type_extension(Path::new("/a/part.v2"), &t[1], &t), PathBuf::from("/a/part.v2.stl"));
        assert_eq!(type_for_extension(&t, "IGS"), Some(2));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn file_uris_decode() {
        assert_eq!(portal::uri_to_path("file:///home/u/My%20Part.stl"), Some(PathBuf::from("/home/u/My Part.stl")));
        assert_eq!(portal::uri_to_path("file://localhost/tmp/x.igs"), Some(PathBuf::from("/tmp/x.igs")));
    }
}
