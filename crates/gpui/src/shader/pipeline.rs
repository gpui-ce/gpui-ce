//! Custom geometry with generated bindings, clipping, and entry points.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use smallvec::SmallVec;

use super::{
    Library, ShaderError,
    compile::{Lanes, layout, pack, prelude_source, validate_module},
    function::LibraryInner,
    value::{CpuValue, Ty, Val, sealed::Sealed},
};

/// How a pipeline's vertices form triangles.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Topology {
    /// Each vertex after the second forms a triangle with the two before it.
    #[default]
    TriangleStrip,
    /// Every three vertices form a triangle.
    TriangleList,
}

/// Your own geometry, drawn in scene order with GPUI's clipping and opacity.
///
/// Write two plain functions; GPUI generates the entry points, bindings,
/// inter-stage IO, clip-space transform, clipping, and blending:
///
/// ```wgsl
/// // `position` comes first: logical pixels from the draw's origin.
/// struct Varyings { position: vec2<f32>, local: vec2<f32> }
///
/// fn vertex(index: u32, center: vec2<f32>, radius: f32) -> Varyings {
///     let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u)) * 2.0 - 1.0;
///     return Varyings(center + corner * radius, corner);
/// }
///
/// // Returns straight alpha. Instance parameters are optional here.
/// fn fragment(in: Varyings, center: vec2<f32>, radius: f32) -> vec4<f32> {
///     let distance = length(in.local) - 1.0;
///     return vec4<f32>(1.0, 0.6, 0.2, Shape_coverage(distance, fwidth(distance)));
/// }
/// ```
///
/// Each instance's parameters (here a center and a radius) reach both stages,
/// so they never need to travel through varyings.
#[derive(Clone)]
pub struct Pipeline {
    program: Arc<PipelineProgram>,
    vertices: u32,
    topology: Topology,
}

struct PipelineProgram {
    id: u64,
    source: String,
    params: SmallVec<[Ty; 8]>,
    lanes: Box<[Lanes]>,
    slots: u32,
}

impl fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pipeline")
            .field("id", &self.program.id)
            .field("params", &self.program.params)
            .field("vertices", &self.vertices)
            .field("topology", &self.topology)
            .finish()
    }
}

impl Pipeline {
    /// A pipeline from WGSL defining `vertex` and `fragment`. The
    /// [prelude](super::prelude) is in scope.
    pub fn wgsl(source: &str) -> Result<Self, ShaderError> {
        Library::wgsl(source)?.pipeline("vertex", "fragment")
    }

    /// A pipeline from GLSL defining `vertex` and `fragment`.
    pub fn glsl(source: &str) -> Result<Self, ShaderError> {
        Library::glsl(source)?.pipeline("vertex", "fragment")
    }

    /// A pipeline from a `#[wgsl]` module defining `vertex` and `fragment`.
    pub fn module(source: &'static wgsl_rs::Source) -> Result<Self, ShaderError> {
        Library::module(source)?.pipeline("vertex", "fragment")
    }

    /// Draw `count` vertices per instance. The default is a 4-vertex strip:
    /// one quad.
    pub fn vertices(mut self, count: u32, topology: Topology) -> Self {
        self.vertices = count;
        self.topology = topology;
        self
    }

    /// Identity of the generated program.
    pub fn id(&self) -> u64 {
        self.program.id
    }

    /// The generated WGSL: entry points `pipeline_vertex` and
    /// `pipeline_fragment` plus the namespaced source. Renderers link it
    /// against the prelude and their glue.
    pub fn source(&self) -> &str {
        &self.program.source
    }

    /// Vertices per instance.
    pub fn vertex_count(&self) -> u32 {
        self.vertices
    }

    /// How vertices form triangles.
    pub fn topology(&self) -> Topology {
        self.topology
    }

    /// Parameter slots each instance occupies, excluding GPUI's own slot.
    pub fn instance_slots(&self) -> u32 {
        self.program.slots
    }

    /// Pack instances for this pipeline, returning the slots and the count.
    pub(crate) fn pack<I: Instance>(
        &self,
        instances: impl IntoIterator<Item = I>,
    ) -> Result<(Arc<[[f32; 4]]>, u32), ShaderError> {
        let slots = self.program.slots as usize;
        let mut packed = Vec::new();
        let mut values = SmallVec::<[Val; 8]>::new();
        let mut count = 0;
        for instance in instances {
            values.clear();
            instance.write(&mut values);
            let types: SmallVec<[Ty; 8]> = values.iter().map(|value| value.ty()).collect();
            if types != self.program.params {
                return Err(ShaderError::Source(format!(
                    "pipeline instances take {:?}, not {types:?}",
                    self.program.params
                )));
            }
            if !values.iter().all(|value| value.is_finite()) {
                return Err(ShaderError::NonFinite);
            }
            packed.resize(packed.len() + slots, [0.0; 4]);
            let start = packed.len() - slots;
            pack(&values, &self.program.lanes, &mut packed[start..]);
            count += 1;
        }
        Ok((packed.into(), count))
    }
}

/// A pipeline's interface, reflected from its `vertex` and `fragment` functions.
struct Interface {
    vertex: String,
    fragment: String,
    /// Instance parameters, in order.
    instance: SmallVec<[Ty; 8]>,
    /// The (namespaced) varyings struct, and its members after `position`.
    varyings: String,
    members: SmallVec<[(String, Ty); 8]>,
    /// Whether `fragment` takes the instance parameters too.
    fragment_params: bool,
}

impl Library {
    /// A [`Pipeline`] from this source's `vertex` and `fragment` functions.
    pub fn pipeline(&self, vertex: &str, fragment: &str) -> Result<Pipeline, ShaderError> {
        let interface = self.reflect(vertex, fragment)?;
        let (lanes, slots) = layout(interface.instance.iter().copied());
        let mut source = interface.generate(&lanes, slots);
        source.push_str(&self.0.chunk);

        const GLUE: &str = "
fn pipeline_slot(index: u32) -> vec4<f32> { return vec4<f32>(0.0); }
fn pipeline_header(index: u32) -> u32 { return index; }
fn pipeline_clip(position: vec2<f32>, header: u32) -> vec4<f32> { return vec4<f32>(position, 0.0, 1.0); }
fn pipeline_position(clip: vec4<f32>, header: u32) -> vec2<f32> { return clip.xy; }
fn pipeline_output(color: vec4<f32>, clip: vec4<f32>, header: u32) -> vec4<f32> { return color; }
";
        validate_module(&format!("{}{GLUE}{source}", prelude_source()))?;

        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Ok(Pipeline {
            program: Arc::new(PipelineProgram {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                source,
                params: interface.instance,
                lanes: lanes.into(),
                slots,
            }),
            vertices: 4,
            topology: Topology::TriangleStrip,
        })
    }

    fn reflect(&self, vertex: &str, fragment: &str) -> Result<Interface, ShaderError> {
        let fail = |message: String| Err(ShaderError::Source(message));
        let LibraryInner { module, prefix, .. } = &*self.0;
        let fragment_struct = format!("{prefix}Fragment");
        let ty = |handle| Ty::from_naga(module, handle, &fragment_struct);
        let find = |name: &str| {
            let full = format!("{prefix}{name}");
            module
                .functions
                .iter()
                .map(|(_, function)| function)
                .find(|function| function.name.as_deref() == Some(full.as_str()))
                .ok_or_else(|| ShaderError::Source(format!("no function `{name}`")))
        };
        let params = |arguments: &[naga::FunctionArgument], name: &str| {
            arguments
                .iter()
                .map(|argument| ty(argument.ty).filter(|ty| *ty != Ty::Fragment))
                .collect::<Option<SmallVec<[Ty; 8]>>>()
                .ok_or_else(|| {
                    ShaderError::Source(format!(
                        "`{name}` parameters must be f32, bool, or vecN<f32>"
                    ))
                })
        };

        let vertex_fn = find(vertex)?;
        let Some((index, instance)) = vertex_fn.arguments.split_first() else {
            return fail(format!("`{vertex}` must take the vertex index (u32) first"));
        };
        if module.types[index.ty].inner != naga::TypeInner::Scalar(naga::Scalar::U32) {
            return fail(format!("`{vertex}` must take the vertex index (u32) first"));
        }
        let instance = params(instance, vertex)?;
        let Some(varyings) = vertex_fn.result.as_ref().map(|result| result.ty) else {
            return fail(format!("`{vertex}` must return a struct"));
        };
        let naga::Type {
            name: Some(varyings_name),
            inner: naga::TypeInner::Struct { members, .. },
        } = &module.types[varyings]
        else {
            return fail(format!("`{vertex}` must return a struct"));
        };
        let members = members
            .iter()
            .map(|member| {
                let ty = ty(member.ty).filter(|ty| !matches!(ty, Ty::Bool | Ty::Fragment))?;
                Some((member.name.clone()?, ty))
            })
            .collect::<Option<SmallVec<[(String, Ty); 8]>>>();
        let Some(members) = members else {
            return fail("varyings must be f32 or vecN<f32>".into());
        };
        if members.first() != Some(&("position".into(), Ty::Vec2)) {
            return fail("varyings must begin with `position: vec2<f32>` (logical pixels)".into());
        }
        if members.len() > 15 {
            return fail("varyings are limited to 15 members".into());
        }

        let fragment_fn = find(fragment)?;
        let fragment_returns_color =
            fragment_fn.result.as_ref().and_then(|result| ty(result.ty)) == Some(Ty::Vec4);
        let Some((input, fragment_params)) = fragment_fn.arguments.split_first() else {
            return fail(format!("`{fragment}` must take the varyings first"));
        };
        if input.ty != varyings || !fragment_returns_color {
            let varyings = varyings_name
                .strip_prefix(prefix.as_str())
                .unwrap_or(varyings_name);
            return fail(format!(
                "`{fragment}` must take `{varyings}` first and return vec4<f32>"
            ));
        }
        let fragment_params = params(fragment_params, fragment)?;
        if !fragment_params.is_empty() && fragment_params != instance {
            return fail(format!(
                "`{fragment}` must take all of `{vertex}`'s instance parameters, or none"
            ));
        }

        Ok(Interface {
            vertex: format!("{prefix}{vertex}"),
            fragment: format!("{prefix}{fragment}"),
            instance,
            varyings: varyings_name.clone(),
            members: members.into_iter().skip(1).collect(),
            fragment_params: !fragment_params.is_empty(),
        })
    }
}

impl Interface {
    /// Entry points around the user's functions. Instance `i` occupies
    /// `slots + 1` slots from `i * (slots + 1)`, the last holding its draw's
    /// header index; see `pipeline_glue` in `gpui_render`.
    fn generate(&self, lanes: &[Lanes], slots: u32) -> String {
        let Self {
            vertex,
            fragment,
            varyings,
            ..
        } = self;
        let stride = slots + 1;
        let fields: String = self
            .members
            .iter()
            .enumerate()
            .map(|(index, (name, ty))| {
                format!("    @location({}) v_{name}: {},\n", index + 1, ty.wgsl())
            })
            .collect();
        let loads: String = lanes
            .iter()
            .zip(&self.instance)
            .enumerate()
            .map(|(index, (lanes, ty))| {
                let swizzle = lanes.swizzle().map(|s| format!(".{s}")).unwrap_or_default();
                let read = format!("pipeline_slot(base + {}u){swizzle}", lanes.slot);
                let value = if *ty == Ty::Bool {
                    format!("{read} != 0.0")
                } else {
                    read
                };
                format!("    let p{index} = {value};\n")
            })
            .collect();
        let params: String = (0..self.instance.len())
            .map(|index| format!(", p{index}"))
            .collect();
        let (fragment_loads, fragment_params) = if self.fragment_params {
            (loads.as_str(), params.as_str())
        } else {
            ("", "")
        };
        let members = |from: &str, prefix: &str| -> String {
            self.members
                .iter()
                .map(|(name, _)| format!(", {from}.{prefix}{name}"))
                .collect()
        };
        let outputs = members("v", "");
        let inputs = members("in", "v_");
        format!(
            "struct PipelineVaryings {{
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) instance: u32,
{fields}}};

@vertex
fn pipeline_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> PipelineVaryings {{
    let base = instance * {stride}u;
    let header = pipeline_header(base + {slots}u);
{loads}    let v = {vertex}(vertex{params});
    return PipelineVaryings(pipeline_clip(v.position, header), instance{outputs});
}}

@fragment
fn pipeline_fragment(in: PipelineVaryings) -> @location(0) vec4<f32> {{
    let base = in.instance * {stride}u;
    let header = pipeline_header(base + {slots}u);
{fragment_loads}    let v = {varyings}(pipeline_position(in.clip, header){inputs});
    return pipeline_output({fragment}(v{fragment_params}), in.clip, header);
}}

"
        )
    }
}

mod seal {
    pub trait Instance {}
}

/// One instance of a [`Pipeline`]: a tuple of [`CpuValue`] parameters.
pub trait Instance: seal::Instance {
    #[doc(hidden)]
    fn write(self, values: &mut SmallVec<[Val; 8]>);
}

macro_rules! instances {
    ($(($($value:ident $index:tt),*))*) => {$(
        impl<$($value: CpuValue),*> seal::Instance for ($($value,)*) {}
        impl<$($value: CpuValue),*> Instance for ($($value,)*) {
            #[allow(unused_variables)]
            fn write(self, values: &mut SmallVec<[Val; 8]>) {
                $(values.push(self.$index.into_value().into_val());)*
            }
        }
    )*};
}

instances! {
    ()
    (A 0)
    (A 0, B 1)
    (A 0, B 1, C 2)
    (A 0, B 1, C 2, D 3)
    (A 0, B 1, C 2, D 3, E 4)
    (A 0, B 1, C 2, D 3, E 4, F 5)
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6)
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPARK: &str = "
    struct Varyings { position: vec2<f32>, local: vec2<f32> }

    fn vertex(index: u32, center: vec2<f32>, radius: f32, tint: vec4<f32>) -> Varyings {
        let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u)) * 2.0 - 1.0;
        return Varyings(center + corner * radius, corner);
    }

    fn fragment(in: Varyings, center: vec2<f32>, radius: f32, tint: vec4<f32>) -> vec4<f32> {
        let distance = length(in.local) - 1.0;
        return vec4<f32>(tint.rgb, tint.a * Shape_coverage(distance, fwidth(distance)));
    }
    ";

    #[test]
    fn pipeline_instances_follow_the_reflected_parameter_layout() {
        let pipeline = Pipeline::wgsl(SPARK).unwrap();
        assert_eq!(pipeline.instance_slots(), 2);
        let (packed, count) = pipeline
            .pack([
                ([10.0, 20.0], 4.0, [1.0, 0.5, 0.0, 1.0]),
                ([30.0, 40.0], 8.0, [0.0, 0.5, 1.0, 0.5]),
            ])
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            &*packed,
            &[
                [10.0, 20.0, 4.0, 0.0],
                [1.0, 0.5, 0.0, 1.0],
                [30.0, 40.0, 8.0, 0.0],
                [0.0, 0.5, 1.0, 0.5],
            ]
        );
        assert!(pipeline.pack([(1.0,)]).is_err());
        assert_eq!(
            pipeline
                .pack([([0.0, 0.0], f32::NAN, [1.0; 4])])
                .unwrap_err(),
            ShaderError::NonFinite
        );

        let missing = Pipeline::wgsl(SPARK.split("fn fragment").next().unwrap());
        assert!(
            matches!(missing, Err(ShaderError::Source(message)) if message.contains("fragment"))
        );
    }
}
