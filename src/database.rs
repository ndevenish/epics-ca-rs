#![allow(dead_code)]

use num::{traits::ToBytes, NumCast};
use std::{
    cmp,
    collections::HashMap,
    convert::TryFrom,
    fmt::Debug,
    io::{Cursor, Write},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::messages::ErrorCondition;

#[derive(Debug, Clone)]
pub struct Limits<T> {
    upper: Option<T>,
    lower: Option<T>,
}

impl<T> Limits<T> {
    fn convert_to<U>(&self) -> Result<Limits<U>, ErrorCondition>
    where
        U: NumCast,
        T: Copy + NumCast,
    {
        // If we have no value, return that. Otherwise, try to convert
        // and return an error if that fails.
        Ok(Limits {
            upper: match self.upper {
                None => Ok(None),
                Some(v) => Ok(Some(U::from(v).ok_or(ErrorCondition::NoConvert)?)),
            }?,
            lower: match self.lower {
                None => Ok(None),
                Some(v) => Ok(Some(U::from(v).ok_or(ErrorCondition::NoConvert)?)),
            }?,
        })
    }
}
impl<T> Default for Limits<T> {
    fn default() -> Self {
        Self {
            upper: None,
            lower: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LimitSet<T> {
    display_limits: Limits<T>,
    warning_limits: Limits<T>,
    alarm_limits: Limits<T>,
}

impl<T> LimitSet<T> {
    fn convert_to<U>(&self) -> Result<LimitSet<U>, ErrorCondition>
    where
        U: NumCast,
        T: Copy + NumCast,
    {
        Ok(LimitSet {
            display_limits: self.display_limits.convert_to()?,
            warning_limits: self.warning_limits.convert_to()?,
            alarm_limits: self.alarm_limits.convert_to()?,
        })
    }
}
impl<T> Default for LimitSet<T> {
    fn default() -> Self {
        LimitSet {
            display_limits: Limits::default(),
            warning_limits: Limits::default(),
            alarm_limits: Limits::default(),
        }
    }
}

/// Hold an individual value or group of values
///
/// Allows distinguishing between single values (where settings an array
/// would be an error) and an array of values, where setting with a
/// different vector length might be acceptable.
#[derive(Clone, Debug)]
pub enum SingleOrVec<T>
where
    T: ToBytes + NumCast + Copy,
{
    Single(T),
    Vector(Vec<T>),
}

impl<T> SingleOrVec<T>
where
    T: ToBytes + NumCast + Copy,
{
    /// Encode this value as a byte array
    ///
    /// Only the first `elements` values will be encoded, or the whole
    /// dataset if the elements value is `None`. Note that if elements
    /// is zero then no data will be returned.
    fn as_bytes(&self, elements: Option<usize>) -> Vec<u8> {
        match self {
            Self::Single(val) => val.to_be_bytes().as_ref().to_vec(),
            Self::Vector(vec) => vec
                .iter()
                .take(elements.unwrap_or(vec.len()))
                .flat_map(|f| f.to_be_bytes().as_ref().to_vec())
                .collect(),
        }
    }
    fn get_count(&self) -> usize {
        match self {
            SingleOrVec::Single(_) => 1,
            SingleOrVec::Vector(v) => v.len(),
        }
    }
    /// Convert to an equivalent SingleOrVec for a different type. This
    /// will convert safely e.g. will fail if it cannot be represented
    /// in the new type.
    fn convert_to<U: ToBytes + NumCast + Copy>(&self) -> Result<SingleOrVec<U>, ErrorCondition> {
        Ok(match self {
            Self::Single(val) => {
                SingleOrVec::Single(U::from(*val).ok_or(ErrorCondition::NoConvert)?)
            }

            Self::Vector(vec) => SingleOrVec::Vector(
                vec.iter()
                    .copied()
                    .map(U::from)
                    .map(|x| x.ok_or(ErrorCondition::NoConvert))
                    .collect::<Result<Vec<U>, ErrorCondition>>()?,
            ),
        })
    }
}

#[derive(Debug, Clone)]
pub struct NumericDBR<T>
where
    T: ToBytes + NumCast + Copy,
{
    pub status: i16,
    pub severity: i16,
    /// Only makes sense for FLOAT/DOUBLE, here to try and avoid duplication
    pub precision: Option<u16>,
    pub units: String,
    pub limits: LimitSet<T>,
    pub value: SingleOrVec<T>,
    pub last_updated: SystemTime,
}
impl<T> NumericDBR<T>
where
    T: ToBytes + Copy + NumCast,
{
    fn get_count(&self) -> usize {
        self.value.get_count()
    }
    fn convert_to<U: ToBytes + Copy + NumCast>(&self) -> Result<NumericDBR<U>, ErrorCondition> {
        Ok(NumericDBR {
            value: self.value.convert_to()?,
            status: self.status,
            severity: self.severity,
            precision: self.precision,
            units: self.units.clone(),
            last_updated: self.last_updated,
            limits: self.limits.convert_to()?,
        })
    }
}

impl<T> Default for NumericDBR<T>
where
    T: Default + ToBytes + Copy + NumCast,
{
    fn default() -> Self {
        Self {
            status: Default::default(),
            severity: Default::default(),
            precision: Default::default(),
            units: Default::default(),
            limits: Default::default(),
            value: SingleOrVec::Single(T::default()),
            last_updated: SystemTime::now(),
        }
    }
}
#[derive(Debug, Clone)]
pub struct StringDBR {
    status: i16,
    severity: i16,
    value: String,
    last_updated: SystemTime,
}

#[derive(Debug, Clone)]
pub struct EnumDBR {
    status: i16,
    severity: i16,
    strings: HashMap<u16, String>,
    value: u16,
    last_updated: SystemTime,
}

impl EnumDBR {
    fn to_numeric<T: ToBytes + NumCast + Copy>(&self) -> Result<NumericDBR<T>, ErrorCondition> {
        Ok(NumericDBR {
            value: SingleOrVec::Single(NumCast::from(self.value).ok_or(ErrorCondition::NoConvert)?),
            severity: self.severity,
            status: self.status,
            last_updated: self.last_updated,
            precision: None,
            units: String::new(),
            limits: LimitSet::default(),
        })
    }
}

#[derive(Debug, Clone)]
pub enum Dbr {
    Enum(EnumDBR),
    String(StringDBR),
    Char(NumericDBR<i8>),
    Int(NumericDBR<i16>),
    Long(NumericDBR<i32>),
    Float(NumericDBR<f32>),
    Double(NumericDBR<f64>),
}

impl Dbr {
    pub fn get_count(&self) -> usize {
        match self {
            Dbr::Enum(_) => 1,
            Dbr::String(_) => 1,
            Dbr::Char(dbr) => dbr.get_count(),
            Dbr::Int(dbr) => dbr.get_count(),
            Dbr::Long(dbr) => dbr.get_count(),
            Dbr::Float(dbr) => dbr.get_count(),
            Dbr::Double(dbr) => dbr.get_count(),
        }
    }
    pub fn get_value(&self) -> DbrValue {
        match self {
            Dbr::Enum(dbr) => DbrValue::Enum(dbr.value),
            Dbr::String(dbr) => DbrValue::String(dbr.value.clone()),
            Dbr::Char(dbr) => DbrValue::Char(dbr.value.clone()),
            Dbr::Int(dbr) => DbrValue::Int(dbr.value.clone()),
            Dbr::Long(dbr) => DbrValue::Long(dbr.value.clone()),
            Dbr::Float(dbr) => DbrValue::Float(dbr.value.clone()),
            Dbr::Double(dbr) => DbrValue::Double(dbr.value.clone()),
        }
    }
    pub fn get_native_type(&self) -> DBRType {
        DBRType {
            basic_type: match self {
                Dbr::Enum(_) => DBRBasicType::Enum,
                Dbr::String(_) => DBRBasicType::String,
                Dbr::Char(_) => DBRBasicType::Char,
                Dbr::Int(_) => DBRBasicType::Int,
                Dbr::Long(_) => DBRBasicType::Long,
                Dbr::Float(_) => DBRBasicType::Float,
                Dbr::Double(_) => DBRBasicType::Double,
            },
            category: DBRCategory::Basic,
        }
    }
    fn get_status(&self) -> (i16, i16) {
        match self {
            Dbr::Enum(dbr) => (dbr.status, dbr.severity),
            Dbr::String(dbr) => (dbr.status, dbr.severity),
            Dbr::Char(dbr) => (dbr.status, dbr.severity),
            Dbr::Int(dbr) => (dbr.status, dbr.severity),
            Dbr::Long(dbr) => (dbr.status, dbr.severity),
            Dbr::Float(dbr) => (dbr.status, dbr.severity),
            Dbr::Double(dbr) => (dbr.status, dbr.severity),
        }
    }
    fn get_last_updated(&self) -> SystemTime {
        match self {
            Dbr::Enum(dbr) => dbr.last_updated,
            Dbr::String(dbr) => dbr.last_updated,
            Dbr::Char(dbr) => dbr.last_updated,
            Dbr::Int(dbr) => dbr.last_updated,
            Dbr::Long(dbr) => dbr.last_updated,
            Dbr::Float(dbr) => dbr.last_updated,
            Dbr::Double(dbr) => dbr.last_updated,
        }
    }

    pub fn convert_to(&self, basic_type: DBRBasicType) -> Result<Dbr, ErrorCondition> {
        Ok(match basic_type {
            DBRBasicType::Char => match self {
                Dbr::Char(val) => Dbr::Char(val.clone()),
                Dbr::Int(val) => Dbr::Char(val.convert_to()?),
                Dbr::Long(val) => Dbr::Char(val.convert_to()?),
                Dbr::Float(val) => Dbr::Char(val.convert_to()?),
                Dbr::Double(val) => Dbr::Char(val.convert_to()?),
                Dbr::String(_) => return Err(ErrorCondition::NoConvert),
                Dbr::Enum(val) => Dbr::Char(val.to_numeric::<i8>()?.convert_to()?),
            },
            DBRBasicType::Int => match self {
                Dbr::Char(val) => Dbr::Int(val.convert_to()?),
                Dbr::Int(val) => Dbr::Int(val.clone()),
                Dbr::Long(val) => Dbr::Int(val.convert_to()?),
                Dbr::Float(val) => Dbr::Int(val.convert_to()?),
                Dbr::Double(val) => Dbr::Int(val.convert_to()?),
                Dbr::String(_) => return Err(ErrorCondition::NoConvert),
                Dbr::Enum(val) => Dbr::Int(val.to_numeric::<i16>()?.convert_to()?),
            },
            DBRBasicType::Long => match self {
                Dbr::Char(val) => Dbr::Long(val.convert_to()?),
                Dbr::Int(val) => Dbr::Long(val.convert_to()?),
                Dbr::Long(val) => Dbr::Long(val.clone()),
                Dbr::Float(val) => Dbr::Long(val.convert_to()?),
                Dbr::Double(val) => Dbr::Long(val.convert_to()?),
                Dbr::String(_) => return Err(ErrorCondition::NoConvert),
                Dbr::Enum(val) => Dbr::Long(val.to_numeric::<i32>()?.convert_to()?),
            },
            DBRBasicType::Float => match self {
                Dbr::Char(val) => Dbr::Float(val.convert_to()?),
                Dbr::Int(val) => Dbr::Float(val.convert_to()?),
                Dbr::Long(val) => Dbr::Float(val.convert_to()?),
                Dbr::Float(val) => Dbr::Float(val.clone()),
                Dbr::Double(val) => Dbr::Float(val.convert_to()?),
                Dbr::String(_) => return Err(ErrorCondition::NoConvert),
                Dbr::Enum(val) => Dbr::Float(val.to_numeric::<f32>()?.convert_to()?),
            },
            DBRBasicType::Double => match self {
                Dbr::Char(val) => Dbr::Double(val.convert_to()?),
                Dbr::Int(val) => Dbr::Double(val.convert_to()?),
                Dbr::Long(val) => Dbr::Double(val.convert_to()?),
                Dbr::Float(val) => Dbr::Double(val.convert_to()?),
                Dbr::Double(val) => Dbr::Double(val.clone()),
                Dbr::String(_) => return Err(ErrorCondition::NoConvert),
                Dbr::Enum(val) => Dbr::Double(val.to_numeric::<f64>()?.convert_to()?),
            },
            DBRBasicType::String => return Err(ErrorCondition::UnavailInServ),
            DBRBasicType::Enum => match self {
                Dbr::Enum(val) => Dbr::Enum(val.clone()),
                _ => return Err(ErrorCondition::NoConvert),
            },
        })
    }

    pub fn encode_value(
        &self,
        data_type: DBRType,
        data_count: usize,
    ) -> Result<(usize, Vec<u8>), ErrorCondition> {
        let mut metadata = Cursor::new(Vec::new());
        // Status, severity always come first, if requested
        if data_type.category != DBRCategory::Basic {
            // Write the status metadata
            let (status, severity) = self.get_status();
            metadata.write_all(&status.to_be_bytes()).unwrap();
            metadata.write_all(&severity.to_be_bytes()).unwrap();
        }
        // Only TIME category writes timestamp information
        if data_type.category == DBRCategory::Time {
            let unix_time = self.get_last_updated().duration_since(UNIX_EPOCH).unwrap();

            let time_s = unix_time.as_secs() as i32 - 631152000i32;
            let time_ns = unix_time.subsec_nanos();
            metadata.write_all(&time_s.to_be_bytes()).unwrap();
            metadata.write_all(&time_ns.to_be_bytes()).unwrap();
        }
        // For now, we don't understand the CTRL structures well enough
        if data_type.category == DBRCategory::Control {
            return Err(ErrorCondition::BadType);
        }
        if data_type.category == DBRCategory::Graphics {
            // Enum, String are special... handle those later
            match data_type.basic_type {
                DBRBasicType::Enum | DBRBasicType::String => {
                    println!("Ignoring request for graphical string or enum");
                    return Err(ErrorCondition::BadType);
                }
                _ => {}
            }
        }
        // Handle insertion of padding
        metadata
            .write_all(&vec![0u8; data_type.get_metadata_padding()])
            .unwrap();

        // Finally... fetching of raw data. Let's start by doing all the
        // matching here, as we don't need to worry about types to hold
        // the cross-conversions.
        let converted = self.convert_to(data_type.basic_type)?;
        let (count, value_data) = converted.get_value().encode_value(if data_count == 0 {
            None
        } else {
            Some(data_count)
        });
        metadata.write_all(&value_data).unwrap();

        // Handle extension of data to be a multiple of 8
        let extra_bytes = metadata.get_ref().len() % 8;
        if 0 < extra_bytes && extra_bytes < 8 {
            metadata.write_all(&vec![0u8; 8 - extra_bytes]).unwrap();
        }

        Ok((count, metadata.into_inner()))
    }
}

#[derive(Clone, Debug)]
pub enum DbrValue {
    Enum(u16),
    String(String),
    Char(SingleOrVec<i8>),
    Int(SingleOrVec<i16>),
    Long(SingleOrVec<i32>),
    Float(SingleOrVec<f32>),
    Double(SingleOrVec<f64>),
}

impl DbrValue {
    fn get_count(&self) -> usize {
        match self {
            DbrValue::Enum(_) => 1,
            DbrValue::String(_) => unimplemented!(),
            DbrValue::Char(val) => val.get_count(),
            DbrValue::Int(val) => val.get_count(),
            DbrValue::Long(val) => val.get_count(),
            DbrValue::Float(val) => val.get_count(),
            DbrValue::Double(val) => val.get_count(),
        }
    }
    /// Encode the value contents of a DBR into a byte vector
    ///
    /// If max_elems is zero, then no data will be returned. If it is
    /// `None`, then all data will be returned.
    fn encode_value(&self, max_elems: Option<usize>) -> (usize, Vec<u8>) {
        let elements = if let Some(max_elem) = max_elems {
            cmp::min(max_elem, self.get_count())
        } else {
            self.get_count()
        };

        (
            elements,
            match self {
                DbrValue::Enum(val) => val.to_be_bytes().to_vec(),
                DbrValue::String(_) => unimplemented!(),
                DbrValue::Char(val) => val.as_bytes(Some(elements)),
                DbrValue::Int(val) => val.as_bytes(Some(elements)),
                DbrValue::Long(val) => val.as_bytes(Some(elements)),
                DbrValue::Float(val) => val.as_bytes(Some(elements)),
                DbrValue::Double(val) => val.as_bytes(Some(elements)),
            },
        )
    }
}
/// Basic DBR Data types, independent of category
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DBRBasicType {
    String = 0,
    Int = 1,
    Float = 2,
    Enum = 3,
    Char = 4,
    Long = 5,
    Double = 6,
}
impl TryFrom<u16> for DBRBasicType {
    type Error = ();
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            x if x == Self::String as u16 => Ok(Self::String),
            x if x == Self::Int as u16 => Ok(Self::Int),
            x if x == Self::Float as u16 => Ok(Self::Float),
            x if x == Self::Enum as u16 => Ok(Self::Enum),
            x if x == Self::Char as u16 => Ok(Self::Char),
            x if x == Self::Long as u16 => Ok(Self::Long),
            x if x == Self::Double as u16 => Ok(Self::Double),
            _ => Err(()),
        }
    }
}
/// Mapping of DBR categories
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DBRCategory {
    Basic = 0,
    Status = 1,
    Time = 2,
    Graphics = 3,
    Control = 4,
}
impl TryFrom<u16> for DBRCategory {
    type Error = ();
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            x if x == Self::Basic as u16 => Ok(Self::Basic),
            x if x == Self::Status as u16 => Ok(Self::Status),
            x if x == Self::Time as u16 => Ok(Self::Time),
            x if x == Self::Graphics as u16 => Ok(Self::Graphics),
            x if x == Self::Control as u16 => Ok(Self::Control),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DBRType {
    pub basic_type: DBRBasicType,
    pub category: DBRCategory,
}

pub const DBR_BASIC_STRING: DBRType = DBRType {
    basic_type: DBRBasicType::String,
    category: DBRCategory::Basic,
};

impl TryFrom<u16> for DBRType {
    type Error = ();
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Ok(Self {
            basic_type: (value % 7).try_into()?,
            category: (value / 7).try_into()?,
        })
    }
}

impl From<DBRType> for u16 {
    fn from(value: DBRType) -> Self {
        value.category as u16 * 7 + value.basic_type as u16
    }
}

impl DBRType {
    /// Give the lookup for the padding for each DBR type
    ///
    /// When encoding a return packet, there is a datatype-specific
    /// padding to be inserted between the metadata about the value and
    /// the actual value itself. This is given as a lookup table rather
    /// than a calculations.
    ///
    /// See https://docs.epics-controls.org/en/latest/internal/ca_protocol.html#payload-data-types
    fn get_metadata_padding(&self) -> usize {
        match (self.category, self.basic_type) {
            (DBRCategory::Status, DBRBasicType::Char) => 1,
            (DBRCategory::Status, DBRBasicType::Double) => 4,
            (DBRCategory::Time, DBRBasicType::Int) => 2,
            (DBRCategory::Time, DBRBasicType::Enum) => 2,
            (DBRCategory::Time, DBRBasicType::Char) => 3,
            (DBRCategory::Time, DBRBasicType::Double) => 4,
            (DBRCategory::Graphics, DBRBasicType::Float) => 2,
            (DBRCategory::Graphics, DBRBasicType::Char) => 1,
            (DBRCategory::Control, DBRBasicType::Char) => 1,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn single_or_vec() {
        let v = SingleOrVec::Single(500i32);
        assert!(v.convert_to::<i16>().is_ok());
        assert!(v.convert_to::<i8>().is_err());
        assert_eq!(v.as_bytes(None), vec![0x00, 0x00, 0x01, 0xF4]);
        assert_eq!(
            v.convert_to::<i16>().unwrap().as_bytes(None),
            vec![0x01, 0xF4]
        );

        let data = vec![500.23f32, 12.7f32];
        let v = SingleOrVec::Vector(data.clone());
        assert_eq!(v.get_count(), 2);
        assert_eq!(
            v.as_bytes(None),
            data.iter()
                .flat_map(|v| v.to_be_bytes())
                .collect::<Vec<u8>>()
        );
        assert_eq!(
            v.as_bytes(Some(1)),
            data.iter()
                .take(1)
                .flat_map(|v| v.to_be_bytes())
                .collect::<Vec<u8>>()
        );
        // Try converting this to an int with truncation
        let v = v.convert_to::<i16>().unwrap();
        assert_eq!(v.as_bytes(None), vec![0x01, 0xf4, 0x00, 0x0c]);

        assert_eq!(
            SingleOrVec::Single(455.9f32)
                .convert_to::<i32>()
                .unwrap()
                .as_bytes(Some(5)),
            vec![0x00, 0x00, 0x01, 0xc7]
        );
    }

    #[test]
    fn encode_dbr() {
        let example_packet = [
            0x0, 0x0, 0x0, 0x0, 0x42, 0x32, 0x19, 0x99, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2a,
        ];
        let dbr = Dbr::Long(NumericDBR {
            value: SingleOrVec::Single(42i32),
            last_updated: SystemTime::UNIX_EPOCH
                .checked_add(Duration::from_secs(1741731609))
                .unwrap(),
            ..Default::default()
        });
        let (_size, out_data) = dbr
            .encode_value(
                DBRType {
                    basic_type: DBRBasicType::Long,
                    category: DBRCategory::Time,
                },
                0,
            )
            .unwrap();
        assert_eq!(out_data.len(), example_packet.len());
        assert_eq!(out_data, example_packet);
    }
}
