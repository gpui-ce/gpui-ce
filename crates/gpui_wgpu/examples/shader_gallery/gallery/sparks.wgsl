struct Spark { position: vec2<f32>, local: vec2<f32> }

fn vertex(index: u32, center: vec2<f32>, radius: f32, tint: vec4<f32>) -> Spark {
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u)) * 2.0 - 1.0;
    return Spark(center + corner * radius, corner);
}

fn fragment(spark: Spark, center: vec2<f32>, radius: f32, tint: vec4<f32>) -> vec4<f32> {
    let glow = exp(-dot(spark.local, spark.local) * 5.0);
    return vec4<f32>(tint.rgb, tint.a * glow);
}
