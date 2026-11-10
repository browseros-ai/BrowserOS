//! Persistence owned by the trace crate. The migrator owns schema changes; TraceBatch is the
//! only row-write path, used by startup recovery, salvage and the ordered writer task.

pub(crate) mod entities;
pub(crate) mod record_rows;
#[cfg(test)]
pub(crate) mod test_faults;
mod trace_batch;
pub(crate) mod trace_migrator;

pub(crate) use trace_batch::{
    OperationEndRow, OperationStartRow, RunCode, RunEndRow, RunStartRow, SalvagedRun, TraceBatch,
};
