// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::arrays::{
    get_bool_array, get_i32_array, get_string_array, get_u16_array, get_u8_array,
    NullableArrayAccessor,
};
use crate::error;
use crate::otlp::related_data::RelatedData;
use crate::schema::consts;
use arrow::array::{
    Array, ArrayRef, BooleanArray, Int32Array, RecordBatch, StringArray, StructArray, UInt16Array,
    UInt32Array, UInt8Array,
};
use arrow::datatypes::{DataType, Field, Fields};
use num_enum::TryFromPrimitive;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::common::v1::InstrumentationScope;
use opentelemetry_proto::tonic::metrics::v1::metric;
use snafu::{OptionExt, ResultExt};

#[derive(Copy, Clone, Eq, PartialEq, Debug, TryFromPrimitive)]
#[repr(u8)]
pub enum MetricType {
    Empty = 0,
    Gauge = 1,
    Sum = 2,
    Histogram = 3,
    ExponentialHistogram = 4,
    Summary = 5,
}

struct ResourceArrays<'a> {
    id: &'a UInt16Array,
    dropped_attributes_count: &'a UInt32Array,
    schema_url: &'a StringArray,
}

impl<'a> ResourceArrays<'a> {
    fn data_type() -> DataType {
        DataType::Struct(Fields::from(vec![
            Field::new(consts::ID, DataType::UInt16, true),
            Field::new(consts::DROPPED_ATTRIBUTES_COUNT, DataType::UInt32, true),
            Field::new(consts::SCHEMA_URL, DataType::Utf8, true),
        ]))
    }
}

impl<'a> TryFrom<&'a RecordBatch> for ResourceArrays<'a> {
    type Error = error::Error;

    fn try_from(rb: &'a RecordBatch) -> Result<Self, Self::Error> {
        let struct_array = Downcaster {
            name: consts::RESOURCE,
            source: rb,
            array: |rb: &'a RecordBatch| rb.column_by_name(consts::RESOURCE),
            expect_type: Self::data_type,
        }
        .downcast::<StructArray>()?;

        let id_array = Downcaster {
            name: consts::ID,
            source: struct_array,
            array: |s: &'a StructArray| s.column_by_name(consts::ID),
            expect_type: || DataType::UInt16,
        }
        .downcast::<UInt16Array>()?;

        let dropped_attributes_count = Downcaster {
            name: consts::DROPPED_ATTRIBUTES_COUNT,
            source: struct_array,
            array: |s: &'a StructArray| s.column_by_name(consts::DROPPED_ATTRIBUTES_COUNT),
            expect_type: || DataType::UInt32,
        }
        .downcast::<UInt32Array>()?;

        let schema_url = Downcaster {
            name: consts::SCHEMA_URL,
            source: struct_array,
            array: |s: &'a StructArray| s.column_by_name(consts::SCHEMA_URL),
            expect_type: || DataType::Utf8,
        }
        .downcast::<StringArray>()?;

        Ok(Self {
            id: id_array,
            dropped_attributes_count,
            schema_url,
        })
    }
}

struct ScopeArrays<'a> {
    name: &'a StringArray,
    version: &'a StringArray,
    dropped_attributes_count: &'a UInt32Array,
    id: &'a UInt16Array,
}

impl<'a> ScopeArrays<'a> {
    fn data_type() -> DataType {
        DataType::Struct(Fields::from(vec![
            Field::new(consts::NAME, DataType::Utf8, true),
            Field::new(consts::VERSION, DataType::Utf8, true),
            Field::new(consts::DROPPED_ATTRIBUTES_COUNT, DataType::UInt32, true),
            Field::new(consts::ID, DataType::UInt16, true),
        ]))
    }
}

struct Downcaster<S, F> {
    name: &'static str,
    source: S,
    array: F,
    expect_type: fn() -> DataType,
}

impl<'a, S, F> Downcaster<S, F> {
    fn downcast<'s, A>(self) -> error::Result<&'a A>
    where
        A: Array + 'static,
        F: Fn(S) -> Option<&'a ArrayRef>,
        S: 'a,
    {
        let array =
            (self.array)(self.source).context(error::ColumnNotFoundSnafu { name: self.name })?;
        array
            .as_any()
            .downcast_ref::<A>()
            .with_context(|| error::ColumnDataTypeMismatchSnafu {
                name: self.name,
                expect: (self.expect_type)(),
                actual: array.data_type().clone(),
            })
    }
}

impl<'a> TryFrom<&'a RecordBatch> for ScopeArrays<'a> {
    type Error = error::Error;

    fn try_from(rb: &'a RecordBatch) -> Result<Self, Self::Error> {
        let scope_array = Downcaster {
            name: consts::SCOPE,
            source: rb,
            array: |rb: &'a RecordBatch| rb.column_by_name(consts::SCOPE),
            expect_type: Self::data_type,
        }
        .downcast::<StructArray>()?;

        let name = Downcaster {
            name: consts::NAME,
            source: scope_array,
            array: |s: &'a StructArray| s.column_by_name(consts::NAME),
            expect_type: || DataType::Utf8,
        }
        .downcast::<StringArray>()?;

        let version = Downcaster {
            name: consts::VERSION,
            source: scope_array,
            array: |s: &'a StructArray| s.column_by_name(consts::VERSION),
            expect_type: || DataType::Utf8,
        }
        .downcast::<StringArray>()?;

        let dropped_attributes_count = Downcaster {
            name: consts::DROPPED_ATTRIBUTES_COUNT,
            source: scope_array,
            array: |s: &'a StructArray| s.column_by_name(consts::DROPPED_ATTRIBUTES_COUNT),
            expect_type: || DataType::UInt32,
        }
        .downcast::<UInt32Array>()?;

        let id = Downcaster {
            name: consts::ID,
            source: scope_array,
            array: |s: &'a StructArray| s.column_by_name(consts::ID),
            expect_type: || DataType::UInt16,
        }
        .downcast::<UInt16Array>()?;

        Ok(Self {
            name,
            version,
            dropped_attributes_count,
            id,
        })
    }
}

struct MetricsArrays<'a> {
    id: &'a UInt16Array,
    metric_type: &'a UInt8Array,
    schema_url: &'a StringArray,
    name: &'a StringArray,
    description: &'a StringArray,
    unit: &'a StringArray,
    aggregation_temporality: &'a Int32Array,
    is_monotonic: &'a BooleanArray,
}

impl<'a> TryFrom<&'a RecordBatch> for MetricsArrays<'a> {
    type Error = error::Error;

    fn try_from(rb: &'a RecordBatch) -> Result<Self, Self::Error> {
        let id = get_u16_array(rb, consts::ID)?;
        let metric_type = get_u8_array(rb, consts::METRIC_TYPE)?;
        let name = get_string_array(rb, consts::NAME)?;
        let description = get_string_array(rb, consts::DESCRIPTION)?;
        let schema_url = get_string_array(rb, consts::SCHEMA_URL)?;
        let unit = get_string_array(rb, consts::UNIT)?;
        let aggregation_temporality = get_i32_array(rb, consts::AGGREGATION_TEMPORALITY)?;
        let is_monotonic = get_bool_array(rb, consts::IS_MONOTONIC)?;
        Ok(Self {
            id,
            metric_type,
            name,
            description,
            schema_url,
            unit,
            aggregation_temporality,
            is_monotonic,
        })
    }
}

/// Builds [ExportMetricsServiceRequest] from given record batch.
pub fn metrics_from(
    rb: &RecordBatch,
    related_data: &mut RelatedData,
) -> error::Result<ExportMetricsServiceRequest> {
    let mut metrics = ExportMetricsServiceRequest::default();

    let mut prev_res_id: Option<u16> = None;
    let mut prev_scope_id: Option<u16> = None;

    let mut res_id = 0;
    let mut scope_id = 0;

    let resource_arrays = ResourceArrays::try_from(rb)?;
    let scope_arrays = ScopeArrays::try_from(rb)?;
    let metrics_arrays = MetricsArrays::try_from(rb)?;

    for idx in 0..rb.num_rows() {
        let res_delta_id = resource_arrays.id.value_at(idx).unwrap_or_default();
        res_id += res_delta_id;

        if prev_res_id != Some(res_id) {
            // new resource id
            prev_res_id = Some(res_id);
            let res_metrics = metrics.resource_metrics.append_and_get();
            prev_scope_id = None;

            // Update the resource field of current resource metrics.
            let resource = res_metrics.resource.get_or_insert_default();
            if let Some(dropped_attributes_count) =
                resource_arrays.dropped_attributes_count.value_at(idx)
            {
                resource.dropped_attributes_count = dropped_attributes_count;
            }

            if let Some(res_id) = resource_arrays.id.value_at(idx)
                && let Some(attrs) = related_data.res_attr_map_store.attribute_by_id(res_id)
            {
                resource.attributes = attrs.to_vec();
            }
            res_metrics.schema_url = resource_arrays.schema_url.value_at(idx).unwrap_or_default();
        }

        let scope_delta_id = scope_arrays.id.value_at(idx).unwrap_or_default();
        scope_id += scope_delta_id;

        if prev_scope_id != Some(scope_id) {
            prev_scope_id = Some(scope_id);
            // safety: We must have appended at least one resource metrics when reach here
            let current_scope_metrics_slice =
                &mut metrics.resource_metrics.last_mut().unwrap().scope_metrics;
            let scope_metrics = current_scope_metrics_slice.append_and_get();

            let mut scope: InstrumentationScope = InstrumentationScope::default();
            scope.name = scope_arrays.name.value_at_or_default(idx);
            scope.version = scope_arrays.version.value_at_or_default(idx);
            scope.dropped_attributes_count = scope_arrays
                .dropped_attributes_count
                .value_at_or_default(idx);
            if let Some(scope_id) = scope_arrays.id.value_at(idx)
                && let Some(attrs) = related_data.scope_attr_map_store.attribute_by_id(scope_id)
            {
                scope.attributes = attrs.to_vec();
            }
            scope_metrics.scope = Some(scope);
            // ScopeMetrics uses the schema_url from metrics arrays.
            scope_metrics.schema_url = metrics_arrays.schema_url.value_at(idx).unwrap_or_default();
        }

        // Creates a metric at the end of current scope metrics slice.
        // safety: we've append at least one value at each slice when reach here.
        let current_scope_metrics = &mut metrics
            .resource_metrics
            .last_mut()
            .unwrap()
            .scope_metrics
            .last_mut()
            .unwrap();
        let current_metric = current_scope_metrics.metrics.append_and_get();
        let delta_id = metrics_arrays.id.value_at_or_default(idx);
        let metric_id = related_data.metric_id_from_delta(delta_id);
        let metric_type_val = metrics_arrays.metric_type.value_at_or_default(idx);
        let metric_type =
            MetricType::try_from(metric_type_val).context(error::UnrecognizedMetricTypeSnafu {
                metric_type: metric_type_val,
            })?;

        let aggregation_temporality = metrics_arrays
            .aggregation_temporality
            .value_at_or_default(idx);
        let is_monotonic = metrics_arrays.is_monotonic.value_at_or_default(idx);
        current_metric.name = metrics_arrays.name.value_at_or_default(idx);
        current_metric.description = metrics_arrays.description.value_at_or_default(idx);
        current_metric.unit = metrics_arrays.unit.value_at_or_default(idx);

        match metric_type {
            MetricType::Gauge => {
                let dps = related_data
                    .number_data_points_store
                    .get_or_default(metric_id);
                current_metric.data = Some(metric::Data::Gauge(
                    opentelemetry_proto::tonic::metrics::v1::Gauge {
                        data_points: std::mem::take(dps),
                    },
                ));
            }
            MetricType::Sum => {
                let dps = related_data
                    .number_data_points_store
                    .get_or_default(metric_id);
                let sum = opentelemetry_proto::tonic::metrics::v1::Sum {
                    data_points: std::mem::take(dps),
                    aggregation_temporality,
                    is_monotonic,
                };
                current_metric.data = Some(metric::Data::Sum(sum));
            }
            MetricType::Histogram => {
                let dps = related_data
                    .histogram_data_points_store
                    .get_or_default(metric_id);
                let histogram = opentelemetry_proto::tonic::metrics::v1::Histogram {
                    data_points: std::mem::take(dps),
                    aggregation_temporality,
                };
                current_metric.data = Some(metric::Data::Histogram(histogram));
            }
            MetricType::ExponentialHistogram => {
                let dps = related_data
                    .e_histogram_data_points_store
                    .get_or_default(metric_id);
                let e_histogram = opentelemetry_proto::tonic::metrics::v1::ExponentialHistogram {
                    data_points: std::mem::take(dps),
                    aggregation_temporality,
                };
                current_metric.data = Some(metric::Data::ExponentialHistogram(e_histogram));
            }
            MetricType::Summary => {
                let dps = related_data
                    .summary_data_points_store
                    .get_or_default(metric_id);
                let summary = opentelemetry_proto::tonic::metrics::v1::Summary {
                    data_points: std::mem::take(dps),
                };
                current_metric.data = Some(metric::Data::Summary(summary));
            }
            MetricType::Empty => return error::EmptyMetricTypeSnafu.fail(),
        }
    }

    Ok(metrics)
}

pub trait AppendAndGet<T> {
    fn append_and_get(&mut self) -> &mut T;
}

impl<T> AppendAndGet<T> for Vec<T>
where
    T: Default,
{
    fn append_and_get(&mut self) -> &mut T {
        self.push(T::default());
        self.last_mut().unwrap()
    }
}
