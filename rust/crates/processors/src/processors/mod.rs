//! One module per processor, named after it the way AutoPkg names its Python
//! modules: `URLDownloader` is in `url_downloader`, `PkgCreator` in
//! `pkg_creator`, and so on. Each module's `execute` runs that processor.
//! Code that several processors share stays in the crate's other modules.

use plist::Dictionary;

/// Writes one of the processor's messages at a verbosity level, to the
/// stream the caller chose for this run.
pub(crate) type Output<'a> = &'a dyn Fn(&Dictionary, i64, String);

pub(crate) mod adobe_acrobat_pro_update_info_provider;
pub(crate) mod adobe_flash_url_provider;
pub(crate) mod adobe_reader_repackager;
pub(crate) mod adobe_reader_url_provider;
pub(crate) mod app_dmg_versioner;
pub(crate) mod app_pkg_creator;
pub(crate) mod autopkg_source_finder;
pub(crate) mod barebones_url_provider;
pub(crate) mod chocolatey_packager;
pub(crate) mod code_signature_verifier;
pub(crate) mod copier;
pub(crate) mod deprecation_warning;
pub(crate) mod dmg_creator;
pub(crate) mod dmg_mounter;
pub(crate) mod end_of_check_phase;
pub(crate) mod file_creator;
pub(crate) mod file_finder;
pub(crate) mod file_mover;
pub(crate) mod find_and_replace;
pub(crate) mod flat_pkg_packer;
pub(crate) mod flat_pkg_unpacker;
pub(crate) mod generate_relocatable_python;
pub(crate) mod github_releases_info_provider;
pub(crate) mod install_from_dmg;
pub(crate) mod installer;
pub(crate) mod make_catalogs_processor;
pub(crate) mod mozilla_url_provider;
pub(crate) mod ms_office_mac_url_and_update_info_provider;
pub(crate) mod munki_catalog_builder;
pub(crate) mod package_required;
pub(crate) mod path_deleter;
pub(crate) mod pkg_copier;
pub(crate) mod pkg_creator;
pub(crate) mod pkg_extractor;
pub(crate) mod pkg_info_creator;
pub(crate) mod pkg_payload_unpacker;
pub(crate) mod pkg_root_creator;
pub(crate) mod plist_editor;
pub(crate) mod plist_reader;
pub(crate) mod puppetlabs_products_url_provider;
pub(crate) mod sassafras_k2_client_customizer;
pub(crate) mod sign_tool_verifier;
pub(crate) mod sparkle_update_info_provider;
pub(crate) mod stop_processing_if;
pub(crate) mod symlinker;
pub(crate) mod unarchiver;
pub(crate) mod url_downloader;
pub(crate) mod url_downloader_python;
pub(crate) mod url_getter;
pub(crate) mod url_text_searcher;
pub(crate) mod variable_setter;
pub(crate) mod versioner;

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// The module name for a processor: its name in snake case, with GitHub
    /// and AutoPkg as single words and `URLand` read as "URL and".
    fn module_name(processor: &str) -> String {
        let name = processor
            .replace("GitHub", "Github")
            .replace("AutoPkg", "Autopkg")
            .replace("URLand", "UrlAnd");
        let characters: Vec<char> = name.chars().collect();
        let mut module = String::new();
        for (index, &character) in characters.iter().enumerate() {
            if character.is_ascii_uppercase() && index > 0 {
                let previous = characters[index - 1];
                let next_is_lower = characters
                    .get(index + 1)
                    .is_some_and(char::is_ascii_lowercase);
                if previous.is_ascii_lowercase()
                    || previous.is_ascii_digit()
                    || (previous.is_ascii_uppercase() && next_is_lower)
                {
                    module.push('_');
                }
            }
            module.push(character.to_ascii_lowercase());
        }
        module
    }

    #[test]
    fn module_names_follow_processor_names() {
        assert_eq!(module_name("URLDownloader"), "url_downloader");
        assert_eq!(module_name("InstallFromDMG"), "install_from_dmg");
        assert_eq!(
            module_name("MSOfficeMacURLandUpdateInfoProvider"),
            "ms_office_mac_url_and_update_info_provider"
        );
        assert_eq!(
            module_name("SassafrasK2ClientCustomizer"),
            "sassafras_k2_client_customizer"
        );
    }

    #[test]
    fn every_processor_has_a_module_named_after_it() {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for &processor in crate::supported() {
            // A recipe repository's path to a processor that's also built in.
            if processor.contains('/') {
                continue;
            }
            let module = format!("{}.rs", module_name(processor));
            // The Munki crate holds the Munki processors, except the
            // deprecated MunkiCatalogBuilder, which only warns.
            let here = crates.join("processors/src/processors").join(&module);
            let munki = crates.join("munki/src/processors").join(&module);
            assert!(
                here.is_file() || munki.is_file(),
                "{processor} has no module named {module}"
            );
        }
    }
}
