//! Headless test for the edits/flags transfer bundle. Point XDG_DATA_HOME at
//! a scratch dir to work on a throwaway catalog.
//!
//!   cargo run --example metadata_cli -- export <out.edmeta>
//!   cargo run --example metadata_cli -- plan   <in.edmeta>
//!   cargo run --example metadata_cli -- apply  <in.edmeta> [--overwrite]
//!                                              [--no-edits|--no-flags|--no-collections]

use effective_darkroom::catalog::Catalog;
use effective_darkroom::metadata_transfer::{
    self, ApplyOptions, OnConflict,
};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        anyhow::bail!("usage: metadata_cli export|plan|apply <file> [flags]");
    }
    let cat = Catalog::open_default()?;
    let path = std::path::PathBuf::from(args.get(1).cloned().unwrap_or_default());

    match args[0].as_str() {
        "export" => {
            let bundle = metadata_transfer::export(&cat, "metadata_cli")?;
            metadata_transfer::write_file(&path, &bundle)?;
            println!(
                "wrote {} of {} photos to {}",
                bundle.photos.len(),
                bundle.source_photo_count,
                path.display()
            );
        }
        "plan" => {
            let bundle = metadata_transfer::read_file(&path)?;
            let plan = metadata_transfer::plan(&cat, &bundle)?;
            println!(
                "photos={} matched={} unmatched={} conflicts={} new_collections={:?}",
                plan.photos,
                plan.matched,
                plan.unmatched,
                plan.conflicts,
                plan.new_collections
            );
        }
        "apply" => {
            let bundle = metadata_transfer::read_file(&path)?;
            let flag = |name: &str| args.iter().any(|a| a == name);
            let opts = ApplyOptions {
                edits: !flag("--no-edits"),
                flags: !flag("--no-flags"),
                collections: !flag("--no-collections"),
                on_conflict: if flag("--overwrite") {
                    OnConflict::Overwrite
                } else {
                    OnConflict::Skip
                },
            };
            let out = metadata_transfer::apply(&cat, &bundle, opts)?;
            println!(
                "edits={} flags={} collections_created={} memberships={} \
                 skipped_conflicts={} unmatched={} errors={:?}",
                out.edits_applied,
                out.flags_applied,
                out.collections_created,
                out.memberships_added,
                out.skipped_conflicts,
                out.unmatched,
                out.errors
            );
        }
        other => anyhow::bail!("unknown command {other}"),
    }
    Ok(())
}
