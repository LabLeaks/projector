/**
@module PROJECTOR.EDGE.DOCTOR_REPORT
Owns doctor finding severity, summary reduction, and final terminal rendering for explicit setup diagnostics.
*/
// @fileimplements PROJECTOR.EDGE.DOCTOR_REPORT
use super::DoctorContext;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DoctorSeverity {
    Warning,
    Error,
}

impl DoctorSeverity {
    fn as_str(self) -> &'static str {
        match self {
            DoctorSeverity::Warning => "warning",
            DoctorSeverity::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DoctorFinding {
    severity: DoctorSeverity,
    message: String,
}

impl DoctorFinding {
    pub(super) fn warning(message: String) -> Self {
        Self {
            severity: DoctorSeverity::Warning,
            message,
        }
    }

    pub(super) fn error(message: String) -> Self {
        Self {
            severity: DoctorSeverity::Error,
            message,
        }
    }
}

pub(super) fn print_summary(_context: &DoctorContext, findings: Vec<DoctorFinding>) {
    let error_count = findings
        .iter()
        .filter(|finding| finding.severity == DoctorSeverity::Error)
        .count();
    let warning_count = findings
        .iter()
        .filter(|finding| finding.severity == DoctorSeverity::Warning)
        .count();
    let doctor_status = if error_count > 0 {
        "error"
    } else if warning_count > 0 {
        "warn"
    } else {
        "ok"
    };

    println!("doctor_status: {}", doctor_status);
    println!("doctor_error_count: {}", error_count);
    println!("doctor_warning_count: {}", warning_count);
    for finding in findings {
        println!("doctor_{}: {}", finding.severity.as_str(), finding.message);
    }
}
