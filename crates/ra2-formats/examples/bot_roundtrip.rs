fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("source.bot");
    let bot = ra2_formats::bot::decode(&std::fs::read(&path).unwrap()).unwrap();
    println!(
        "{}: damaged_model={} faces={} paint={}",
        bot.name,
        bot.damaged_model.is_some(),
        bot.chassis.face_count,
        bot.chassis.texture.is_some()
    );
    if let Some(model) = &bot.chassis.model {
        let o = &model.objects[0];
        let area = o
            .faces
            .iter()
            .map(|f| {
                let [a, b, c] = f.vertices.map(|i| o.vertices[i]);
                (b - a).cross(c - a).length() * 0.5
            })
            .sum::<f32>();
        println!(
            "Chassis area={area} density={} mass={}",
            bot.chassis.surface_density,
            area * bot.chassis.surface_density
        );
    }
    if let Some(out) = args.next() {
        std::fs::write(out, ra2_formats::bot::encode(&bot).unwrap()).unwrap();
    }
}
