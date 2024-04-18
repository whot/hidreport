// SPDX-License-Identifier: MIT
//
// FIXME: remove this once we have something that doesn't scream hundreds of warnings
#![allow(unused_variables)]
#![allow(dead_code)]

use std::ops::{Range, RangeInclusive};
use std::collections::HashMap;
use thiserror::Error;

pub mod hid;
pub mod hut;
pub mod types;

use hid::*;
pub use types::*;

#[derive(Debug)]
pub struct ReportDescriptor {
    pub input_reports: Vec<Report>,
    pub output_reports: Vec<Report>,
    pub feature_reports: Vec<Report>,
}

impl ReportDescriptor {}

#[derive(Copy, Clone, Debug)]
pub enum Direction {
    Input,
    Output,
    Feature,
}

#[derive(Debug)]
pub struct Report {
    /// The report ID, if any
    pub id: Option<u8>,
    /// The size of this report in bits
    pub size: usize,
    /// The fields present in this report
    pub items: Vec<Field>,
    pub direction: Direction,
}

#[derive(Clone, Copy, Debug)]
pub struct Usage {
    usage_page: UsagePage,
    usage_id: UsageId,
}

#[derive(Clone, Copy, Debug)]
pub struct LogicalRange {
    minimum: LogicalMinimum,
    maximum: LogicalMaximum,
}

#[derive(Clone, Copy, Debug)]
pub struct PhysicalRange {
    minimum: PhysicalMinimum,
    maximum: PhysicalMaximum,
}

#[derive(Debug)]
pub enum Field {
    Variable(VariableField),
    Array(ArrayField),
    Constant(ConstantField),
}

impl Field {
    fn bits(&self) -> &RangeInclusive<usize>  {
        match self {
            Field::Variable(f) => &f.bits,
            Field::Array(f) => &f.bits,
            Field::Constant(f) => &f.bits,
        }
    }

    fn report_id(&self) -> &Option<ReportId>  {
        match self {
            Field::Variable(f) => &f.report_id,
            Field::Array(f) => &f.report_id,
            Field::Constant(f) => &f.report_id,
        }
    }
}

#[derive(Debug)]
pub struct VariableField {
    usage: Usage,
    bits: RangeInclusive<usize>,
    logical_range: LogicalRange,
    physical_range: Option<PhysicalRange>,
    unit: Option<Unit>,
    unit_exponent: Option<UnitExponent>,
    collections: Vec<Collection>,
    report_id: Option<ReportId>,
    direction: Direction,
}

#[derive(Debug)]
pub struct ArrayField {
    usages: Vec<Usage>,
    bits: RangeInclusive<usize>,
    logical_range: LogicalRange,
    physical_range: Option<PhysicalRange>,
    unit: Option<Unit>,
    unit_exponent: Option<UnitExponent>,
    collections: Vec<Collection>,
    report_id: Option<ReportId>,
    direction: Direction,
}

#[derive(Debug)]
pub struct ConstantField {
    bits: RangeInclusive<usize>,
    report_id: Option<ReportId>,
    direction: Direction,
}

#[derive(Copy, Clone, Debug)]
pub struct Collection(u8);

#[derive(Error, Debug)]
pub enum ParserError {
    #[error("Invalid data {data} at offset {offset}: {message}")]
    InvalidData {
        offset: u32,
        data: u32,
        message: String,
    },
    #[error("Parsing would lead to out-of-bounds")]
    OutOfBounds,
}

type Result<T> = std::result::Result<T, ParserError>;

impl TryFrom<&[u8]> for ReportDescriptor {
    type Error = ParserError;

    fn try_from(bytes: &[u8]) -> Result<ReportDescriptor> {
        parse_report_descriptor(bytes)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Globals {
    usage_page: Option<UsagePage>,
    logical_minimum: Option<LogicalMinimum>,
    logical_maximum: Option<LogicalMaximum>,
    physical_minimum: Option<PhysicalMinimum>,
    physical_maximum: Option<PhysicalMaximum>,
    unit_exponent: Option<UnitExponent>,
    unit: Option<Unit>,
    report_size: Option<ReportSize>,
    report_id: Option<ReportId>,
    report_count: Option<ReportCount>,
}

/// Special struct for the [Locals] because the usage_page
/// is optional for those, unlike our [Usage] struct which is
/// the finalized one.
#[derive(Clone, Copy, Debug)]
struct LocalUsage {
    usage_page: Option<UsagePage>,
    usage_id: UsageId,
}

#[derive(Clone, Copy, Debug, Default)]
struct Locals {
    usage: Option<LocalUsage>,
    // FIXME: needs the same LocalUsage treatment
    usage_minimum: Option<UsageMinimum>,
    usage_maximum: Option<UsageMaximum>,
    designator_index: Option<DesignatorIndex>,
    designator_minimum: Option<DesignatorMinimum>,
    designator_maximum: Option<DesignatorMaximum>,
    string_index: Option<StringIndex>,
    string_minimum: Option<StringMinimum>,
    string_maximum: Option<StringMaximum>,
    delimiter: Option<Delimiter>,
}

struct Offsets {
    /// Bit offset for the report-id less report
    bit_offset: usize,
    /// Bit offsets for report with report-id
    bit_offsets: HashMap<ReportId, usize>,
}

impl Offsets {
    fn new() -> Self {
        Self {
            bit_offset: 0,
            bit_offsets: HashMap::default(),
        }
    }
}

#[derive(Debug)]
struct Stack {
    globals: Vec<Globals>,
    locals: Vec<Locals>,
    pub collections: Vec<Collection>,
}

impl Stack {
    fn new() -> Self {
        Stack {
            globals: vec![Globals::default()],
            locals: vec![Locals::default()],
            collections: vec![],
        }
    }

    fn push(&mut self) {
        let current = &self.globals.last().unwrap();
        self.globals.push(**current);

        let current = &self.locals.last().unwrap();
        self.locals.push(**current);
    }

    fn pop(&mut self) {
        self.globals.pop();
        self.locals.pop();
    }

    fn reset_locals(&mut self) {
        self.locals = vec![Locals::default()];
    }

    fn globals(&mut self) -> &mut Globals {
        self.globals.last_mut().unwrap()
    }

    fn locals(&mut self) -> &mut Locals {
        self.locals.last_mut().unwrap()
    }

    // Should be globals and globals_mut but i'd have to 
    // update the update_stack macro for that.
    fn globals_const(&self) -> &Globals {
        self.globals.last().unwrap()
    }

    fn locals_const(&self) -> &Locals {
        self.locals.last().unwrap()
    }
}

fn compile_usages(globals: &Globals, locals: &Locals) -> Vec<Usage> {
    // Prefer UsageMinimum/Maximum over Usage because the latter may be set from an earlier call
    match locals.usage_minimum {
        Some(_) => {
            let min: u32 = locals.usage_minimum.expect("Missing UsageMinimum in locals").into();
            let max: u32 = locals.usage_maximum.expect("Missing UsageMaximum in locals").into();
            let usage_page = globals.usage_page.expect("Missing UsagePage in globals");

            RangeInclusive::new(min, max)
                .map(|u| Usage {
                    usage_page: UsagePage(usage_page.into()),
                    usage_id: UsageId(u as u16),

                })
                .collect()
        },
        None => {
            match locals.usage.as_ref().expect("Missing Usage in locals") {
                // local item's Usage had a Usage Page included
                LocalUsage {
                    usage_page: Some(up),
                    usage_id,
                } => vec![Usage {
                    usage_page: *up,
                    usage_id: *usage_id,
                }],
                // Usage Page comes from the global item
                LocalUsage {
                    usage_page: None,
                    usage_id,
                } => {
                    let usage_page = globals.usage_page.expect("Missing UsagePage in globals");
                    vec![Usage {
                        usage_page,
                        usage_id: *usage_id,
                    }]
                }
            }
        },
    }
}

fn handle_main_item(item: &MainItem, stack: &mut Stack, offsets: &mut Offsets) -> Result<Vec<Field>> {
    let globals = stack.globals_const();
    let locals = stack.locals_const();

    let is_constant = match item {
        MainItem::Input(i) => i.is_constant,
        MainItem::Output(i) => i.is_constant,
        MainItem::Feature(i) => i.is_constant,
        _ => panic!("Invalid item for handle_main_item()"),
    };

    let direction = match item {
        MainItem::Input(i) => Direction::Input,
        MainItem::Output(i) => Direction::Output,
        MainItem::Feature(i) => Direction::Feature,
        _ => panic!("Invalid item for handle_main_item()"),
    };

    let bit_offset: &mut usize = match globals.report_id {
        None => &mut offsets.bit_offset,
        Some(id) => {
            if !offsets.bit_offsets.contains_key(&id) {
                offsets.bit_offsets.insert(id, 0);
            }
            offsets.bit_offsets.get_mut(&id).unwrap()
        }
    };

    let report_id = globals.report_id;
    let report_size = globals.report_size.expect("Missing report size in globals");
    let report_count = globals.report_count.expect("Missing report count in globals");

    if is_constant {
        let nbits = usize::from(report_size) * usize::from(report_count) - 1;
        let bits = RangeInclusive::new(*bit_offset, *bit_offset + nbits);

        *bit_offset += nbits;

        let field = ConstantField {
            bits,
            report_id,
            direction,
        };
        return Ok(vec![Field::Constant(field)]);
    }

    let logical_range = LogicalRange {
        minimum: globals.logical_minimum.expect("Missing LogicalMinimum"),
        maximum: globals.logical_maximum.expect("Missing LogicalMaximum"),
    };

    let physical_range = match (globals.physical_minimum, globals.physical_maximum) {
        (Some(min), Some(max)) => Some(PhysicalRange {
            minimum: globals.physical_minimum.unwrap(),
            maximum: globals.physical_maximum.unwrap(),
        }),
        _ => None,
    };

    let unit = globals.unit;
    let unit_exponent = globals.unit_exponent;

    let is_variable = match item {
        MainItem::Input(i) => i.is_variable,
        MainItem::Output(i) => i.is_variable,
        MainItem::Feature(i) => i.is_variable,
        _ => panic!("Invalid item for handle_main_item()"),
    };

    let usages = compile_usages(globals, locals);
    let collections = stack.collections.clone();
    let field: Vec<Field> = if is_variable {
        Range { start: 0, end: usize::from(report_count) }
            .map(|c| {
                let nbits = usize::from(report_size);
                let bits = RangeInclusive::new(*bit_offset, *bit_offset + nbits - 1);
                *bit_offset += nbits;

                let usage = usages.get(c).or_else(||  usages.last()).unwrap();
                let field = VariableField {
                    usage: *usage,
                    bits,
                    logical_range,
                    physical_range,
                    unit,
                    unit_exponent,
                    collections: collections.clone(),
                    report_id,
                    direction,
                };
                Field::Variable(field)
        }).collect()
    } else {
        let nbits = usize::from(report_size) * usize::from(report_count);
        let bits = RangeInclusive::new(*bit_offset, *bit_offset + nbits -1);

        *bit_offset += nbits;

        let field = ArrayField {
            usages,
            bits,
            logical_range,
            physical_range,
            unit,
            unit_exponent,
            collections,
            report_id,
            direction,
        };

        vec![Field::Array(field)]
    };

    Ok(field)
}

macro_rules! update_stack {
    ($stack:ident, $class:ident, $which:ident, $from:ident) => {
        //println!("Updating {} with value {:?}", stringify!($which), &$from);
        let state = $stack.$class();
        state.$which = Some($from);
    };
}

fn parse_report_descriptor(bytes: &[u8]) -> Result<ReportDescriptor> {
    let items = hid::ReportDescriptorItems::try_from(bytes)?;

    let mut stack = Stack::new();
    let mut offsets = Offsets::new();

    let mut fields: Vec<Field> = items.iter().flat_map(|rdesc_item| {
        let item = rdesc_item.item();
        match item.item_type() {
            ItemType::Main(MainItem::Collection(i)) => {
                let c = Collection(u8::from(&i));
                stack.collections.push(c);
            }
            ItemType::Main(MainItem::EndCollection) => {
                stack.collections.pop();
            }
            ItemType::Main(item) => {
                let fields = handle_main_item(&item, &mut stack, &mut offsets).expect("main item parsing failed");
                stack.reset_locals();
                return Some(fields);
            }
            ItemType::Long => {}
            ItemType::Reserved => {}
            ItemType::Global(GlobalItem::UsagePage { usage_page }) => {
                update_stack!(stack, globals, usage_page, usage_page);
            }
            ItemType::Global(GlobalItem::LogicalMinimum { minimum }) => {
                update_stack!(stack, globals, logical_minimum, minimum);
            }
            ItemType::Global(GlobalItem::LogicalMaximum { maximum }) => {
                update_stack!(stack, globals, logical_maximum, maximum);
            }
            ItemType::Global(GlobalItem::PhysicalMinimum { minimum }) => {
                update_stack!(stack, globals, physical_minimum, minimum);
            }
            ItemType::Global(GlobalItem::PhysicalMaximum { maximum }) => {
                update_stack!(stack, globals, physical_maximum, maximum);
            }
            ItemType::Global(GlobalItem::UnitExponent { exponent }) => {
                update_stack!(stack, globals, unit_exponent, exponent);
            }
            ItemType::Global(GlobalItem::Unit { unit }) => {
                update_stack!(stack, globals, unit, unit);
            }
            ItemType::Global(GlobalItem::ReportSize { size }) => {
                update_stack!(stack, globals, report_size, size);
            }
            ItemType::Global(GlobalItem::ReportId { id }) => {
                update_stack!(stack, globals, report_id, id);
            }
            ItemType::Global(GlobalItem::ReportCount { count }) => {
                update_stack!(stack, globals, report_count, count);
            }
            ItemType::Global(GlobalItem::Push) => {
                stack.push();
            }
            ItemType::Global(GlobalItem::Pop) => {
                stack.pop();
            }
            ItemType::Global(GlobalItem::Reserved) => {}
            ItemType::Local(LocalItem::Usage {
                usage_page,
                usage_id,
            }) => {
                let usage = LocalUsage {
                    usage_page,
                    usage_id,
                };
                update_stack!(stack, locals, usage, usage);
            }
            ItemType::Local(LocalItem::UsageMinimum { minimum }) => {
                update_stack!(stack, locals, usage_minimum, minimum);
            }
            ItemType::Local(LocalItem::UsageMaximum { maximum }) => {
                update_stack!(stack, locals, usage_maximum, maximum);
            }
            ItemType::Local(LocalItem::DesignatorIndex { index }) => {
                update_stack!(stack, locals, designator_index, index);
            }
            ItemType::Local(LocalItem::DesignatorMinimum { minimum }) => {
                update_stack!(stack, locals, designator_minimum, minimum);
            }
            ItemType::Local(LocalItem::DesignatorMaximum { maximum }) => {
                update_stack!(stack, locals, designator_maximum, maximum);
            }
            ItemType::Local(LocalItem::StringIndex { index }) => {
                update_stack!(stack, locals, string_index, index);
            }
            ItemType::Local(LocalItem::StringMinimum { minimum }) => {
                update_stack!(stack, locals, string_minimum, minimum);
            }
            ItemType::Local(LocalItem::StringMaximum { maximum }) => {
                update_stack!(stack, locals, string_maximum, maximum);
            }
            ItemType::Local(LocalItem::Delimiter { delimiter }) => {
                update_stack!(stack, locals, delimiter, delimiter);
            }
            ItemType::Local(LocalItem::Reserved { value: u8 }) => {}
        };
        None
    })
    .flatten()
    .collect();

    // Sort by report ID (if any)
    fields.sort_by(|a, b| {
        let r1 = a.report_id();
        let r2 = b.report_id();

        match (r1, r2) {
            (None, None) => std::cmp::Ordering::Equal,
            (Some(a), Some(b)) => {
                let aid = u8::from(a);
                let bid = u8::from(b);
                aid.cmp(&bid)
            },
            _ => panic!("All reports must have a report ID"),
        }
    });

    for field in fields {
        println!("{field:?}");
    }

    panic!("FIXME");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {}
}
