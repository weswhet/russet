//! One module per Munki processor, named after it the way AutoPkg names its
//! Python modules: `MunkiImporter` is in `munki_importer`, and so on.

pub mod munki_importer;
pub mod munki_info_creator;
pub mod munki_installs_items_creator;
pub mod munki_optional_receipt_editor;
pub mod munki_pkginfo_merger;
pub mod munki_set_default_catalog;
