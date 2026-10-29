use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    ZhTw,
    EnUs,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::ZhTw, Self::EnUs];

    pub fn code(self) -> &'static str {
        match self {
            Self::ZhTw => "zh-tw",
            Self::EnUs => "en-us",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::ZhTw => "繁體中文（台灣）",
            Self::EnUs => "English",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.code() == code)
    }
}

fn catalog(language: Language) -> &'static BTreeMap<String, String> {
    static ZH: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    static EN: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    let (cache, source) = match language {
        Language::ZhTw => (&ZH, include_str!("../locales/zh-tw.json")),
        Language::EnUs => (&EN, include_str!("../locales/en-us.json")),
    };
    cache.get_or_init(|| serde_json::from_str(source).expect("Invalid embedded language catalog"))
}

#[derive(Clone, Copy)]
pub struct Translator(pub Language);

impl Translator {
    pub fn get(self, key: &'static str) -> &'static str {
        catalog(self.0)
            .get(key)
            .or_else(|| catalog(Language::ZhTw).get(key))
            .map(String::as_str)
            .unwrap_or(key)
    }

    pub fn format(self, key: &'static str, arguments: &[(&str, String)]) -> String {
        // Replace in a single pass so braces in user data are never interpreted.
        let template = self.get(key);
        let mut result = String::new();
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            result.push_str(&rest[..start]);
            let Some(end) = rest[start..].find('}').map(|end| start + end) else {
                result.push_str(&rest[start..]);
                return result;
            };
            let name = &rest[start + 1..end];
            if let Some((_, value)) = arguments.iter().find(|(key, _)| *key == name) {
                result.push_str(value);
            } else {
                result.push_str(&rest[start..=end]);
            }
            rest = &rest[end + 1..];
        }
        result.push_str(rest);
        result
    }

    pub fn progress(self, message: &str) -> String {
        let key = match message {
            "正在建立 LUKSbox，並驗證資料可重新解鎖…" => "progress.vault_create",
            "正在解鎖 LUKSbox…" => "progress.vault_unlock",
            "正在新增 LUKSbox 解鎖密碼…" => "progress.vault_add_key",
            "請在系統視窗或安全金鑰上完成 Passkey 註冊與兩次驗證…" => {
                "progress.passkey_enroll"
            }
            "請在系統視窗或安全金鑰上完成 Passkey 驗證…" => {
                "progress.passkey_unlock"
            }
            "正在連線並驗證 SSH 主機指紋…" => "progress.ssh",
            "正在執行 apt-get update，只更新套件索引…" => "progress.apt_update",
            "正在讀取待更新套件…" => "progress.apt_list",
            _ => return message.into(),
        };
        self.get(key).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_cover_ui_keys_with_matching_placeholders() {
        fn placeholders(value: &str) -> Vec<&str> {
            let mut names: Vec<_> = value
                .split('{')
                .skip(1)
                .filter_map(|part| part.split_once('}').map(|(name, _)| name))
                .collect();
            names.sort_unstable();
            names
        }
        let zh = catalog(Language::ZhTw);
        let en = catalog(Language::EnUs);
        assert_eq!(zh.keys().collect::<Vec<_>>(), en.keys().collect::<Vec<_>>());
        for (key, value) in zh {
            assert!(
                !value.trim().is_empty() && !en[key].trim().is_empty(),
                "{key}"
            );
            assert_eq!(placeholders(value), placeholders(&en[key]), "{key}");
        }
        for source in [include_str!("ui.rs"), include_str!("theme.rs")] {
            for function in ["t.get(", "t.format("] {
                for part in source.split(function).skip(1) {
                    let key = part
                        .trim_start()
                        .strip_prefix('"')
                        .unwrap()
                        .split('"')
                        .next()
                        .unwrap();
                    assert!(zh.contains_key(key), "Missing UI translation: {key}");
                }
            }
        }
        assert_eq!(
            Translator(Language::EnUs).format("host.delete_prompt", &[("name", "{count}".into())]),
            "Delete the local connection and check results for “{count}”?"
        );
    }
}
