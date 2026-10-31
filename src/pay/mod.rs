//! The finance milestone: what people earn for the hours the agents send.
//!
//! Its position was set when it was planned. The money is computed from the
//! tracker's hours, not from a timesheet taken on trust, and this is not
//! bookkeeping: no taxes, no holiday pay at average earnings, no sick pay. It
//! answers what the hours earned under the terms in force, and says so wherever
//! the law would compute something else.
//!
//! Three inputs make the money. Two existed before this module: the hours
//! worked on each date (`workday_figures`, ADR 0023) and the norm each date
//! owed (ADR 0017). The third is how each person is paid, kept here as a
//! history of versions (ADR 0027).

pub mod terms;
