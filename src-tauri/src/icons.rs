pub fn executable_icon_data_url_with(
    extractor: impl FnOnce(&str) -> Result<String, String>,
    executable_path: &str,
) -> Result<Option<String>, String> {
    if executable_path.trim().is_empty() {
        return Ok(None);
    }

    extractor(executable_path).map(Some)
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_executable_icon(executable_path: String) -> Result<Option<String>, String> {
    executable_icon_data_url_with(
        |path| {
            powershift_windows::png_data_url_from_executable(path)
                .map_err(|error| error.to_string())
        },
        &executable_path,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_paths_skip_icon_extraction() {
        let result = executable_icon_data_url_with(|_| Err("should not run".to_string()), "   ")
            .expect("empty path");

        assert_eq!(result, None);
    }

    #[test]
    fn extractor_result_is_returned_as_optional_data_url() {
        let result =
            executable_icon_data_url_with(|path| Ok(format!("icon:{path}")), "C:\\Game\\game.exe")
                .expect("icon");

        assert_eq!(result.as_deref(), Some("icon:C:\\Game\\game.exe"));
    }
}
