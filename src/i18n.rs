//! Application language selection and embedded Fluent resources.

use i18n_embed::fluent::{fluent_language_loader, FluentLanguageLoader};
use i18n_embed::LanguageLoader;
#[cfg(not(target_arch = "wasm32"))]
use i18n_embed::DesktopLanguageRequester;
#[cfg(target_arch = "wasm32")]
use i18n_embed::WebLanguageRequester;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(RustEmbed)]
#[folder = "locales/"]
struct Localizations;

/// User-selectable UI language. `System` keeps following the platform's
/// preferred locale while explicit choices remain stable across restarts.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en-US")]
    EnUs,
    #[serde(rename = "tr-TR")]
    TrTr,
    #[serde(rename = "nl-NL")]
    NlNl,
    #[serde(rename = "fr-FR")]
    FrFr,
    #[serde(rename = "de-DE")]
    DeDe,
    #[serde(rename = "hi-IN")]
    HiIn,
    #[serde(rename = "ru-RU")]
    RuRu,
    #[serde(rename = "zh-CN")]
    ZhCn,
}

impl Language {
    pub const ALL: [Language; 9] = [
        Language::System,
        Language::EnUs,
        Language::TrTr,
        Language::NlNl,
        Language::FrFr,
        Language::DeDe,
        Language::HiIn,
        Language::RuRu,
        Language::ZhCn,
    ];

    fn requested(self) -> Vec<i18n_embed::unic_langid::LanguageIdentifier> {
        match self {
            Language::System => system_languages(),
            Language::EnUs => vec!["en-US".parse().expect("valid locale")],
            Language::TrTr => vec!["tr-TR".parse().expect("valid locale")],
            Language::NlNl => vec!["nl-NL".parse().expect("valid locale")],
            Language::FrFr => vec!["fr-FR".parse().expect("valid locale")],
            Language::DeDe => vec!["de-DE".parse().expect("valid locale")],
            Language::HiIn => vec!["hi-IN".parse().expect("valid locale")],
            Language::RuRu => vec!["ru-RU".parse().expect("valid locale")],
            Language::ZhCn => vec!["zh-CN".parse().expect("valid locale")],
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Language::System => crate::tr!("language-system"),
            Language::EnUs => crate::tr!("language-english"),
            Language::TrTr => crate::tr!("language-turkish"),
            Language::NlNl => crate::tr!("language-dutch"),
            Language::FrFr => crate::tr!("language-french"),
            Language::DeDe => crate::tr!("language-german"),
            Language::HiIn => crate::tr!("language-hindi"),
            Language::RuRu => crate::tr!("language-russian"),
            Language::ZhCn => crate::tr!("language-chinese-simplified"),
        };
        f.write_str(&label)
    }
}

fn system_languages() -> Vec<i18n_embed::unic_langid::LanguageIdentifier> {
    #[cfg(target_arch = "wasm32")]
    let requested = WebLanguageRequester::requested_languages();
    #[cfg(not(target_arch = "wasm32"))]
    let requested = DesktopLanguageRequester::requested_languages();
    requested
}

fn load_language(
    loader: &FluentLanguageLoader,
    language: Language,
) -> Result<(), i18n_embed::I18nEmbedError> {
    let mut requested = language.requested();
    if requested.is_empty() {
        requested.push(loader.fallback_language().clone());
    }
    i18n_embed::select(loader, &Localizations, &requested).map(|_| ())
}

pub fn loader() -> &'static FluentLanguageLoader {
    static LOADER: OnceLock<FluentLanguageLoader> = OnceLock::new();
    LOADER.get_or_init(|| {
        let loader = fluent_language_loader!();
        if let Err(error) = load_language(&loader, Language::System) {
            eprintln!("Unable to load system UI language: {error}");
            loader
                .load_languages(&Localizations, &[loader.fallback_language().clone()])
                .expect("fallback UI language must be embedded");
        }
        loader
    })
}

/// Apply a user preference process-wide. The loader swaps resources atomically,
/// so the next Iced view pass immediately receives the new language.
pub fn set_language(language: Language) -> Result<(), i18n_embed::I18nEmbedError> {
    load_language(loader(), language)
}

/// Built-in ribbon modules have stable ids; plug-ins keep their supplied title
/// until they provide their own localization bundle.
pub fn ribbon_module_title(id: &str, fallback: &str) -> String {
    match id {
        "draw" => crate::tr!("ribbon-tab-draw"),
        "annotate" => crate::tr!("ribbon-tab-annotate"),
        "insert" => crate::tr!("ribbon-tab-insert"),
        "model" => crate::tr!("ribbon-tab-model"),
        "layout" => crate::tr!("ribbon-tab-layout"),
        "manage" => crate::tr!("ribbon-tab-manage"),
        "view" => crate::tr!("ribbon-tab-view"),
        _ => fallback.to_string(),
    }
}

#[macro_export]
macro_rules! tr {
    ($message_id:literal $(,)?) => {
        i18n_embed_fl::fl!($crate::i18n::loader(), $message_id)
    };
    ($message_id:literal, $($name:ident = $value:expr),+ $(,)?) => {
        i18n_embed_fl::fl!(
            $crate::i18n::loader(),
            $message_id,
            $($name = $value),+
        )
    };
}
