use super::style::{Fill, FillType, Gradient, GradientType, Stroke};
use super::VectorData;
use crate::renderer::GraphicElementRendered;
use crate::transform::{Footprint, Transform, TransformMut};
use crate::{Color, GraphicGroup, Node};
use core::future::Future;

use bezier_rs::{Subpath, SubpathTValue};
use glam::{DAffine2, DVec2};
use num_traits::Zero;

#[derive(Debug, Clone, Copy)]
pub struct SetFillNode<FillType, SolidColor, GradientType, Start, End, Transform, Positions> {
	fill_type: FillType,
	solid_color: SolidColor,
	gradient_type: GradientType,
	start: Start,
	end: End,
	transform: Transform,
	positions: Positions,
}

#[node_macro::node_fn(SetFillNode)]
fn set_vector_data_fill(
	mut vector_data: VectorData,
	fill_type: FillType,
	solid_color: Option<Color>,
	gradient_type: GradientType,
	start: DVec2,
	end: DVec2,
	transform: DAffine2,
	positions: Vec<(f64, Option<Color>)>,
) -> VectorData {
	vector_data.style.set_fill(match fill_type {
		FillType::None | FillType::Solid => solid_color.map_or(Fill::None, Fill::Solid),
		FillType::Gradient => Fill::Gradient(Gradient {
			start,
			end,
			transform,
			positions,
			gradient_type,
		}),
	});
	vector_data
}

#[derive(Debug, Clone, Copy)]
pub struct SetStrokeNode<Color, Weight, DashLengths, DashOffset, LineCap, LineJoin, MiterLimit> {
	color: Color,
	weight: Weight,
	dash_lengths: DashLengths,
	dash_offset: DashOffset,
	line_cap: LineCap,
	line_join: LineJoin,
	miter_limit: MiterLimit,
}

#[node_macro::node_fn(SetStrokeNode)]
fn set_vector_data_stroke(
	mut vector_data: VectorData,
	color: Option<Color>,
	weight: f32,
	dash_lengths: Vec<f32>,
	dash_offset: f32,
	line_cap: super::style::LineCap,
	line_join: super::style::LineJoin,
	miter_limit: f32,
) -> VectorData {
	vector_data.style.set_stroke(Stroke {
		color,
		weight: weight as f64,
		dash_lengths,
		dash_offset: dash_offset as f64,
		line_cap,
		line_join,
		line_join_miter_limit: miter_limit as f64,
	});
	vector_data
}

#[derive(Debug, Clone, Copy)]
pub struct RepeatNode<Direction, Count> {
	direction: Direction,
	count: Count,
}

#[node_macro::node_fn(RepeatNode)]
fn repeat_vector_data(mut vector_data: VectorData, direction: DVec2, count: u32) -> VectorData {
	// repeat the vector data
	let VectorData { subpaths, transform, .. } = &vector_data;

	let mut new_subpaths: Vec<Subpath<_>> = Vec::with_capacity(subpaths.len() * count as usize);
	let inverse = transform.inverse();
	let direction = inverse.transform_vector2(direction);
	for i in 0..count {
		let transform = DAffine2::from_translation(direction * i as f64);
		for mut subpath in subpaths.clone() {
			subpath.apply_transform(transform);
			new_subpaths.push(subpath);
		}
	}

	vector_data.subpaths = new_subpaths;
	vector_data
}

#[derive(Debug, Clone, Copy)]
pub struct CircularRepeatNode<AngleOffset, Radius, Count> {
	angle_offset: AngleOffset,
	radius: Radius,
	count: Count,
}

#[node_macro::node_fn(CircularRepeatNode)]
fn circular_repeat_vector_data(mut vector_data: VectorData, angle_offset: f32, radius: f32, count: u32) -> VectorData {
	let mut new_subpaths: Vec<Subpath<_>> = Vec::with_capacity(vector_data.subpaths.len() * count as usize);

	let bounding_box = vector_data.bounding_box().unwrap();
	let center = (bounding_box[0] + bounding_box[1]) / 2.;

	let base_transform = DVec2::new(0., radius as f64) - center;

	for i in 0..count {
		let angle = (2. * std::f64::consts::PI / count as f64) * i as f64 + angle_offset.to_radians() as f64;
		let rotation = DAffine2::from_angle(angle);
		let transform = DAffine2::from_translation(center) * rotation * DAffine2::from_translation(base_transform);
		for mut subpath in vector_data.subpaths.clone() {
			subpath.apply_transform(transform);
			new_subpaths.push(subpath);
		}
	}

	vector_data.subpaths = new_subpaths;
	vector_data
}

#[derive(Debug, Clone, Copy)]
pub struct BoundingBoxNode;

#[node_macro::node_fn(BoundingBoxNode)]
fn generate_bounding_box(vector_data: VectorData) -> VectorData {
	let bounding_box = vector_data.bounding_box().unwrap();
	VectorData::from_subpaths(vec![Subpath::new_rect(
		vector_data.transform.transform_point2(bounding_box[0]),
		vector_data.transform.transform_point2(bounding_box[1]),
	)])
}

pub trait ConcatElement {
	fn concat(&mut self, other: &Self, transform: DAffine2);
}

impl ConcatElement for VectorData {
	fn concat(&mut self, other: &Self, transform: DAffine2) {
		for mut subpath in other.subpaths.iter().cloned() {
			subpath.apply_transform(transform * other.transform);
			self.subpaths.push(subpath);
		}
		// TODO: properly deal with fills such as gradients
		self.style = other.style.clone();
		self.mirror_angle.extend(other.mirror_angle.iter().copied());
		self.alpha_blending = other.alpha_blending;
	}
}

impl ConcatElement for GraphicGroup {
	fn concat(&mut self, other: &Self, transform: DAffine2) {
		// TODO: Decide if we want to keep this behaviour whereby the layers are flattened
		for mut element in other.iter().cloned() {
			*element.transform_mut() = transform * element.transform() * other.transform();
			self.push(element);
		}
		self.alpha_blending = other.alpha_blending;
	}
}

#[derive(Debug, Clone, Copy)]
pub struct CopyToPoints<Points, Instance> {
	points: Points,
	instance: Instance,
}

#[node_macro::node_fn(CopyToPoints)]
async fn copy_to_points<I: GraphicElementRendered + Default + ConcatElement + TransformMut, FP: Future<Output = VectorData>, FI: Future<Output = I>>(
	footprint: Footprint,
	points: impl Node<Footprint, Output = FP>,
	instance: impl Node<Footprint, Output = FI>,
) -> I {
	let points = self.points.eval(footprint).await;
	let instance = self.instance.eval(footprint).await;

	let points_list = points.subpaths.iter().flat_map(|s| s.anchors());

	let instance_bounding_box = instance.bounding_box(DAffine2::IDENTITY).unwrap_or_default();
	let instance_center = -0.5 * (instance_bounding_box[0] + instance_bounding_box[1]);

	let mut result = I::default();
	for point in points_list {
		let translation = points.transform.transform_point2(point) + instance_center;
		result.concat(&instance, DAffine2::from_translation(translation));
	}

	result
}

#[derive(Debug, Clone, Copy)]
pub struct SamplePoints<Spacing, StartOffset, StopOffset, AdaptiveSpacing> {
	spacing: Spacing,
	start_offset: StartOffset,
	stop_offset: StopOffset,
	adaptive_spacing: AdaptiveSpacing,
}

#[node_macro::node_fn(SamplePoints)]
fn sample_points(mut vector_data: VectorData, spacing: f32, start_offset: f32, stop_offset: f32, adaptive_spacing: bool) -> VectorData {
	let spacing = spacing as f64;
	let start_offset = start_offset as f64;
	let stop_offset = stop_offset as f64;

	for subpath in &mut vector_data.subpaths {
		if subpath.is_empty() || !spacing.is_finite() || spacing <= 0. {
			continue;
		}

		subpath.apply_transform(vector_data.transform);
		let length = subpath.length(None);
		let used_length = length - start_offset - stop_offset;
		if used_length <= 0. {
			continue;
		}
		let used_length_without_remainder = used_length - used_length % spacing;

		let count = if adaptive_spacing {
			(used_length / spacing).round()
		} else {
			(used_length / spacing + f64::EPSILON).floor()
		};

		if count >= 1. {
			let new_anchors = (0..=count as usize).map(|c| {
				let ratio = c as f64 / count;

				// With adaptive spacing, we widen or narrow the points (that's the rounding performed above) as necessary to ensure the last point is always at the end of the path
				// Without adaptive spacing, we just evenly space the points at the exact specified spacing, usually falling short before the end of the path

				let used_length_here = if adaptive_spacing { used_length } else { used_length_without_remainder };
				let t = ratio * used_length_here + start_offset;
				subpath.evaluate(SubpathTValue::GlobalEuclidean(t / length))
			});
			*subpath = Subpath::from_anchors(new_anchors, subpath.closed() && count as usize > 1);
		}

		subpath.apply_transform(vector_data.transform.inverse());
	}
	vector_data
}

#[derive(Debug, Clone, Copy)]
pub struct SplinesFromPointsNode {}

#[node_macro::node_fn(SplinesFromPointsNode)]
fn splines_from_points(mut vector_data: VectorData) -> VectorData {
	for subpath in &mut vector_data.subpaths {
		*subpath = Subpath::new_cubic_spline(subpath.anchors());
	}

	vector_data
}
