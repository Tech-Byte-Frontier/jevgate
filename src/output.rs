use crate::{
    options::Format,
    schema::{FileResult, Report},
};
use anyhow::Result;
use std::io::Write;
pub fn emit(report: &Report, format: Format) -> Result<()> {
    let mut out = std::io::stdout().lock();
    if matches!(format, Format::Json | Format::Jsonl) {
        if format == Format::Json {
            serde_json::to_writer_pretty(&mut out, report)?;
        } else {
            serde_json::to_writer(&mut out, report)?;
        }
        writeln!(out)?;
    } else {
        writeln!(
            out,
            "JevGate: {} · {} files · {} API requests",
            report.status,
            report.files.len(),
            report.api_requests
        )?;
        for error in &report.errors {
            writeln!(out, "Error: {error}")?;
        }
        for file in &report.files {
            emit_file(&mut out, file)?;
        }
    }
    Ok(())
}
pub(super) fn emit_file(out: &mut impl Write, file: &FileResult) -> Result<()> {
    writeln!(
        out,
        "{} [{}]",
        file.path.display(),
        serde_json::to_value(&file.status)?
            .as_str()
            .unwrap_or("unknown")
    )?;
    if let Some(error) = &file.error {
        writeln!(out, "  {error}")?;
    }
    if let Some(roles) = &file.role_assessment {
        writeln!(out, "  Semantic roles only; no maintainability assessment")?;
        for region in &roles.regions {
            writeln!(
                out,
                "  {}:{}–{} ({})",
                region["evidence"]["path"].as_str().unwrap_or(""),
                region["evidence"]["start_line"],
                region["evidence"]["end_line"],
                region["status"].as_str().unwrap_or("unknown")
            )?;
            for (role, _) in crate::roles::ROLES {
                let signal = &region["roles"][role];
                writeln!(
                    out,
                    "    {role}: {} · {:.0}%",
                    signal["status"].as_str().unwrap_or("unknown"),
                    signal["answer"]["noul"].as_f64().unwrap_or(0.0) * 100.0
                )?;
            }
        }
    }
    for (name, d) in &file.dimensions {
        writeln!(
            out,
            "  {name}: {} · concern {:.0}%",
            serde_json::to_value(&d.status)?
                .as_str()
                .unwrap_or("unknown"),
            d.concern_probability * 100.0
        )?;
        writeln!(out, "    {}", d.decision_basis)?;
    }
    Ok(())
}
