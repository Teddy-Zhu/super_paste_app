use std::cmp::Ordering;

type Point = [f32; 2];
pub(super) type BoxPoints = [Point; 4];

#[derive(Clone, Debug)]
pub(super) struct LayoutItem {
    pub(super) points: BoxPoints,
    pub(super) text: String,
    pub(super) confidence: f64,
}

#[derive(Clone, Debug)]
struct TextLine {
    points: BoxPoints,
    text: String,
    confidence: f64,
}

#[derive(Clone, Debug)]
struct ParagraphGroup {
    lines: Vec<TextLine>,
    outer: BoxPoints,
}

#[derive(Clone, Copy)]
struct ReadingDirection {
    inline: &'static str,
    block: &'static str,
}

pub(super) fn arrange(items: Vec<LayoutItem>) -> (String, f64) {
    if items.is_empty() {
        return (String::new(), 0.0);
    }

    let source_angles = items
        .iter()
        .map(|item| box_angle(&item.points))
        .collect::<Vec<_>>();
    let initial = average_angles(&source_angles);
    let aligned = source_angles
        .iter()
        .copied()
        .filter(|angle| angle_aligned(*angle, initial))
        .collect::<Vec<_>>();
    let angle_median = median(&aligned);
    let deviations = aligned
        .iter()
        .map(|angle| (angle - angle_median).abs())
        .collect::<Vec<_>>();
    let deviation_median = median(&deviations);
    let consistent = aligned
        .into_iter()
        .filter(|angle| ((angle - angle_median) / (deviation_median * 1.4826)).abs() < 2.0)
        .collect::<Vec<_>>();
    let inline_angle = normalize_angle(average_angles(&consistent));
    let block_angle = normalize_angle(inline_angle + 90.0);
    let direction = infer_direction(inline_angle, block_angle);
    let angle_inline = angle_vector(closest_direction_angle(
        inline_angle,
        direction_angle(direction.inline),
    ));
    let angle_block = angle_vector(closest_direction_angle(
        block_angle,
        direction_angle(direction.block),
    ));
    let relative_inline = transform_point(angle_inline, "lr", "tb", direction);
    let relative_block = transform_point(angle_block, "lr", "tb", direction);

    let permutation = box_permutation(direction);
    let mut lines = items
        .into_iter()
        .map(|item| {
            let points =
                permutation.map(|index| transform_point(item.points[index], "lr", "tb", direction));
            TextLine {
                points,
                text: item.text,
                confidence: item.confidence,
            }
        })
        .collect::<Vec<_>>();

    lines.sort_by(|left, right| {
        compare(
            block_start(&left.points),
            block_start(&right.points),
            relative_block,
        )
    });

    let mut grouped_lines: Vec<Vec<TextLine>> = Vec::new();
    for line in lines {
        let joins_last = grouped_lines
            .last()
            .and_then(|group| group.last())
            .is_some_and(|last| {
                distance_by_vector(center(&line.points), center(&last.points), relative_block)
                    < 0.5 * block_size(&line.points)
            });
        if joins_last {
            grouped_lines.last_mut().expect("group exists").push(line);
        } else {
            grouped_lines.push(vec![line]);
        }
    }

    let mut merged_lines = Vec::new();
    for mut group in grouped_lines {
        if group.len() == 1 {
            merged_lines.push(group.pop().expect("single line exists"));
            continue;
        }

        let threshold = group
            .iter()
            .map(|line| block_size(&line.points))
            .sum::<f32>()
            / group.len() as f32;
        group.sort_by(|left, right| {
            compare(
                inline_start(&left.points),
                inline_start(&right.points),
                relative_inline,
            )
        });

        let mut current = group.remove(0);
        for next in group {
            let gap = dot(inline_start(&next.points), relative_inline)
                - dot(inline_end(&current.points), relative_inline);
            if gap > threshold {
                merged_lines.push(current);
                current = next;
            } else {
                current.text.push_str(&next.text);
                current.confidence = (current.confidence + next.confidence) / 2.0;
                current.points = union_boxes(
                    &[current.points, next.points],
                    relative_inline,
                    relative_block,
                );
            }
        }
        merged_lines.push(current);
    }

    let confidence = if merged_lines.is_empty() {
        0.0
    } else {
        merged_lines.iter().map(|line| line.confidence).sum::<f64>() / merged_lines.len() as f64
    };
    let fallback_text = merged_lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut paragraphs = cluster_paragraphs(merged_lines, relative_inline, relative_block);
    let mut columns = Vec::new();
    for paragraph in paragraphs.drain(..) {
        let joins_column = columns.iter().position(|column: &Vec<ParagraphGroup>| {
            let previous = column.last().expect("columns are non-empty");
            let threshold = block_size(&paragraph.lines[0].points);
            inline_start_distance(&previous.outer, &paragraph.outer, relative_inline)
                < 3.0 * threshold
                && inline_end_distance(&previous.outer, &paragraph.outer, relative_inline)
                    < 3.0 * threshold
                && block_gap(&paragraph.outer, &previous.outer, relative_block) < 2.1 * threshold
        });
        if let Some(index) = joins_column {
            columns[index].push(paragraph);
        } else {
            columns.push(vec![paragraph]);
        }
    }

    for column in &mut columns {
        column.sort_by(|left, right| {
            compare(
                block_start(&left.outer),
                block_start(&right.outer),
                relative_block,
            )
        });
    }

    let mut sections = columns
        .into_iter()
        .map(|column| {
            let lines = column
                .into_iter()
                .flat_map(|paragraph| paragraph.lines)
                .collect::<Vec<_>>();
            let outer = union_boxes(
                &lines.iter().map(|line| line.points).collect::<Vec<_>>(),
                relative_inline,
                relative_block,
            );
            ParagraphGroup { lines, outer }
        })
        .collect::<Vec<_>>();

    stable_sort_sections(&mut sections, relative_inline, relative_block);
    let mut joined_sections: Vec<ParagraphGroup> = Vec::new();
    for section in sections {
        let Some(previous) = joined_sections.last_mut() else {
            joined_sections.push(section);
            continue;
        };
        let threshold = block_size(&section.lines[0].points);
        let merge = (previous.lines.len() == 1
            && inline_start_distance(&previous.outer, &section.outer, relative_inline)
                < 3.0 * threshold)
            || (section.lines.len() == 1
                && inline_start_distance(&previous.outer, &section.outer, relative_inline)
                    < 3.0 * threshold)
            || (inline_start_distance(&previous.outer, &section.outer, relative_inline)
                < 3.0 * threshold
                && inline_end_distance(&previous.outer, &section.outer, relative_inline)
                    < 3.0 * threshold);
        if merge {
            previous.lines.extend(section.lines);
            previous.outer = union_boxes(
                &previous
                    .lines
                    .iter()
                    .map(|line| line.points)
                    .collect::<Vec<_>>(),
                relative_inline,
                relative_block,
            );
        } else {
            joined_sections.push(section);
        }
    }

    let mut recalculated = false;
    let mut reading_groups: Vec<ParagraphGroup> = Vec::new();
    for section in joined_sections {
        let Some(previous) = reading_groups.last_mut() else {
            reading_groups.push(section);
            continue;
        };
        let threshold = block_size(&section.lines[0].points);
        let merge = compare(
            block_end(&section.outer),
            block_end(&previous.outer),
            relative_block,
        ) == Ordering::Less
            && (inline_start_distance(&previous.outer, &section.outer, relative_inline)
                < 3.0 * threshold
                || inline_end_distance(&previous.outer, &section.outer, relative_inline)
                    < 3.0 * threshold);
        if merge {
            previous.lines.extend(section.lines);
            recalculated = true;
            previous.lines.sort_by(|left, right| {
                compare(
                    block_start(&left.points),
                    block_start(&right.points),
                    relative_block,
                )
            });
            previous.outer = union_boxes(
                &previous
                    .lines
                    .iter()
                    .map(|line| line.points)
                    .collect::<Vec<_>>(),
                relative_inline,
                relative_block,
            );
        } else {
            reading_groups.push(section);
        }
    }
    if recalculated {
        stable_sort_sections(&mut reading_groups, relative_inline, relative_block);
    }

    let mut paragraph_texts = Vec::new();
    for group in &reading_groups {
        paragraph_texts.extend(split_paragraphs(group, relative_inline, relative_block));
    }
    let text = if paragraph_texts.is_empty() {
        fallback_text
    } else {
        paragraph_texts.join("\n")
    };
    (text, confidence)
}

fn cluster_paragraphs(
    mut lines: Vec<TextLine>,
    inline: Point,
    block: Point,
) -> Vec<ParagraphGroup> {
    lines.sort_by(|left, right| {
        compare(block_start(&left.points), block_start(&right.points), block)
    });
    let mut paragraphs: Vec<ParagraphGroup> = Vec::new();
    for line in lines {
        let nearest = paragraphs
            .iter()
            .enumerate()
            .filter_map(|(index, paragraph)| {
                paragraph
                    .lines
                    .last()
                    .map(|last| (index, distance(line.points[0], last.points[0])))
            })
            .min_by(|left, right| left.1.total_cmp(&right.1));
        let Some((index, _)) = nearest else {
            paragraphs.push(ParagraphGroup {
                outer: line.points,
                lines: vec![line],
            });
            continue;
        };

        let last = paragraphs[index]
            .lines
            .last()
            .expect("nearest paragraph has a line");
        let min_inline = inline_size(&line.points).min(inline_size(&last.points));
        let line_height = block_size(&line.points);
        let related = (inline_start_distance(&line.points, &last.points, inline)
            < 3.0 * line_height
            || inline_end_distance(&line.points, &last.points, inline) < 3.0 * line_height
            || distance_by_vector(center(&line.points), center(&last.points), inline)
                < min_inline * 0.4)
            && block_gap(&line.points, &last.points, block) < line_height * 1.1;
        if related {
            let paragraph = &mut paragraphs[index];
            paragraph.lines.push(line);
            paragraph.outer = union_boxes(
                &paragraph
                    .lines
                    .iter()
                    .map(|line| line.points)
                    .collect::<Vec<_>>(),
                inline,
                block,
            );
        } else {
            paragraphs.push(ParagraphGroup {
                outer: line.points,
                lines: vec![line],
            });
        }
    }
    paragraphs
}

fn split_paragraphs(group: &ParagraphGroup, inline: Point, block: Point) -> Vec<String> {
    if group.lines.is_empty() {
        return Vec::new();
    }
    let mut gaps = group
        .lines
        .windows(2)
        .map(|pair| distance_by_vector(center(&pair[1].points), center(&pair[0].points), block))
        .collect::<Vec<_>>();
    gaps.sort_by(f32::total_cmp);
    let average_height = group
        .lines
        .iter()
        .map(|line| block_size(&line.points))
        .sum::<f32>()
        / group.lines.len() as f32;
    let mut gap_groups: Vec<Vec<f32>> = Vec::new();
    for gap in gaps {
        let separates_group = gap_groups
            .last()
            .and_then(|values| values.last())
            .is_some_and(|previous| (previous - gap).abs() >= average_height * 0.5);
        if separates_group {
            gap_groups.push(Vec::new());
            continue;
        }
        if gap_groups.is_empty() {
            gap_groups.push(Vec::new());
        }
        gap_groups.last_mut().expect("gap group exists").push(gap);
    }
    let typical_gap = gap_groups
        .iter()
        .map(|values| values.iter().sum::<f32>() / values.len() as f32)
        .filter(|average| average.is_finite())
        .min_by(f32::total_cmp)
        .unwrap_or(0.0);

    let mut paragraphs = vec![vec![group.lines[0].clone()]];
    let mut previous = &group.lines[0];
    for line in group.lines.iter().skip(1) {
        let indent = inline_start_distance(&previous.points, &group.outer, inline);
        let predicted = add(
            add(inline_start(&previous.points), scale(block, typical_gap)),
            scale(inline, -indent),
        );
        let start = inline_start(&line.points);
        let line_height = block_size(&line.points);
        let split = inline_end_distance(&previous.points, &group.outer, inline) > 2.0 * line_height
            || distance(predicted, start) > 0.5 * line_height;
        if split {
            paragraphs.push(vec![line.clone()]);
        } else {
            paragraphs
                .last_mut()
                .expect("paragraph exists")
                .push(line.clone());
        }
        previous = line;
    }
    paragraphs
        .iter()
        .map(|lines| paragraph_text(lines))
        .collect()
}

fn paragraph_text(lines: &[TextLine]) -> String {
    let mut text = String::new();
    for line in lines {
        let previous = text.chars().last();
        let next = line.text.chars().next();
        if previous.is_some_and(|character| {
            (!is_ideographic(character) && !is_east_asian_punctuation(character))
                || !next.is_some_and(|next| is_ideographic(next) || is_east_asian_punctuation(next))
        }) {
            text.push(' ');
        }
        text.push_str(&line.text);
    }
    text
}

fn stable_sort_sections(sections: &mut [ParagraphGroup], inline: Point, block: Point) {
    for index in 1..sections.len() {
        let mut cursor = index;
        while cursor > 0 {
            let left = &sections[cursor - 1];
            let right = &sections[cursor];
            let threshold = left
                .lines
                .first()
                .map(|line| block_size(&line.points))
                .unwrap_or(2.0);
            let ordering =
                if distance_by_vector(block_start(&left.outer), block_start(&right.outer), block)
                    < threshold
                {
                    compare(
                        inline_start(&left.outer),
                        inline_start(&right.outer),
                        inline,
                    )
                } else {
                    compare(block_start(&left.outer), block_start(&right.outer), block)
                };
            if ordering != Ordering::Greater {
                break;
            }
            sections.swap(cursor - 1, cursor);
            cursor -= 1;
        }
    }
}

fn union_boxes(boxes: &[BoxPoints], inline: Point, block: Point) -> BoxPoints {
    let mut inline_min = f32::INFINITY;
    let mut inline_max = f32::NEG_INFINITY;
    let mut block_min = f32::INFINITY;
    let mut block_max = f32::NEG_INFINITY;
    for point in boxes.iter().flat_map(|box_points| box_points.iter()) {
        let inline_position = dot(*point, inline);
        let block_position = dot(*point, block);
        inline_min = inline_min.min(inline_position);
        inline_max = inline_max.max(inline_position);
        block_min = block_min.min(block_position);
        block_max = block_max.max(block_position);
    }
    let start = add(scale(inline, inline_min), scale(block, block_min));
    let inline_extent = scale(inline, inline_max - inline_min);
    let block_extent = scale(block, block_max - block_min);
    [
        start,
        add(start, inline_extent),
        add(add(start, inline_extent), block_extent),
        add(start, block_extent),
    ]
}

fn infer_direction(inline_angle: f32, block_angle: f32) -> ReadingDirection {
    let inline_axis = if angle_aligned(inline_angle, 0.0) {
        "x"
    } else {
        "y"
    };
    let block_axis = if angle_aligned(block_angle, 90.0) {
        "y"
    } else {
        "x"
    };
    if inline_axis == "y" && block_axis == "x" {
        ReadingDirection {
            inline: "tb",
            block: "rl",
        }
    } else {
        ReadingDirection {
            inline: "lr",
            block: "tb",
        }
    }
}

fn box_permutation(direction: ReadingDirection) -> [usize; 4] {
    let inline = direction_labels(direction.inline);
    let block = direction_labels(direction.block);
    let labels = [
        (inline[0], block[0]),
        (inline[1], block[0]),
        (inline[1], block[1]),
        (inline[0], block[1]),
    ];
    labels.map(|(inline, block)| {
        let key = if matches!(inline, 'l' | 'r') {
            [inline, block]
        } else {
            [block, inline]
        };
        match key {
            ['l', 't'] => 0,
            ['r', 't'] => 1,
            ['r', 'b'] => 2,
            ['l', 'b'] => 3,
            _ => 0,
        }
    })
}

fn direction_labels(direction: &str) -> [char; 2] {
    match direction {
        "lr" => ['l', 'r'],
        "rl" => ['r', 'l'],
        "tb" => ['t', 'b'],
        "bt" => ['b', 't'],
        _ => ['l', 'r'],
    }
}

fn direction_vector(direction: &str) -> Point {
    match direction {
        "lr" => [1.0, 0.0],
        "rl" => [-1.0, 0.0],
        "tb" => [0.0, 1.0],
        "bt" => [0.0, -1.0],
        _ => [1.0, 0.0],
    }
}

fn transform_point(
    point: Point,
    from_inline: &str,
    from_block: &str,
    to: ReadingDirection,
) -> Point {
    let source_inline = direction_vector(from_inline);
    let source_block = direction_vector(from_block);
    let physical = add(
        scale(source_inline, point[0]),
        scale(source_block, point[1]),
    );
    [
        dot(physical, direction_vector(to.inline)),
        dot(physical, direction_vector(to.block)),
    ]
}

fn box_angle(points: &BoxPoints) -> f32 {
    let vector = if inline_size(points) < block_size(points) {
        from_points(
            center_pair(points[2], points[3]),
            center_pair(points[0], points[1]),
        )
    } else {
        from_points(
            center_pair(points[1], points[2]),
            center_pair(points[0], points[3]),
        )
    };
    normalize_angle(vector[1].atan2(vector[0]).to_degrees())
}

fn average_angles(angles: &[f32]) -> f32 {
    let mut average = 0.0;
    for (index, angle) in angles.iter().copied().enumerate() {
        let canonical = if angle > 180.0 { angle - 180.0 } else { angle };
        let opposite = canonical - 180.0;
        let chosen = if index == 0 {
            canonical
        } else if (opposite - average).abs() < (canonical - average).abs() {
            opposite
        } else {
            canonical
        };
        average = (average * index as f32 + chosen) / (index + 1) as f32;
    }
    average
}

fn median(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

fn angle_aligned(angle: f32, center: f32) -> bool {
    (angle - center).abs() < 45.0
        || (angle - (center - 180.0)).abs() < 45.0
        || (angle - 180.0 - center).abs() < 45.0
}

fn closest_direction_angle(angle: f32, target: f32) -> f32 {
    [angle, angle - 360.0, angle - 180.0, angle + 180.0]
        .into_iter()
        .min_by(|left, right| (left - target).abs().total_cmp(&(right - target).abs()))
        .unwrap_or(angle)
}

fn direction_angle(direction: &str) -> f32 {
    match direction {
        "lr" => 0.0,
        "rl" => 180.0,
        "tb" => 90.0,
        "bt" => 270.0,
        _ => 0.0,
    }
}

fn angle_vector(degrees: f32) -> Point {
    let radians = degrees.to_radians();
    [radians.cos(), radians.sin()]
}

fn normalize_angle(angle: f32) -> f32 {
    angle.rem_euclid(360.0)
}

fn inline_start(points: &BoxPoints) -> Point {
    center_pair(points[0], points[3])
}

fn inline_end(points: &BoxPoints) -> Point {
    center_pair(points[1], points[2])
}

fn block_start(points: &BoxPoints) -> Point {
    center_pair(points[0], points[1])
}

fn block_end(points: &BoxPoints) -> Point {
    center_pair(points[2], points[3])
}

fn center(points: &BoxPoints) -> Point {
    center_pair(points[0], points[2])
}

fn inline_size(points: &BoxPoints) -> f32 {
    points[1][0] - points[0][0]
}

fn block_size(points: &BoxPoints) -> f32 {
    points[3][1] - points[0][1]
}

fn inline_start_distance(left: &BoxPoints, right: &BoxPoints, inline: Point) -> f32 {
    distance_by_vector(left[0], right[0], inline)
}

fn inline_end_distance(left: &BoxPoints, right: &BoxPoints, inline: Point) -> f32 {
    distance_by_vector(left[1], right[1], inline)
}

fn block_gap(left: &BoxPoints, right: &BoxPoints, block: Point) -> f32 {
    distance_by_vector(left[0], right[3], block)
}

fn compare(left: Point, right: Point, vector: Point) -> Ordering {
    (dot(left, vector) - dot(right, vector)).total_cmp(&0.0)
}

fn distance_by_vector(left: Point, right: Point, vector: Point) -> f32 {
    (dot(left, vector) - dot(right, vector)).abs()
}

fn center_pair(left: Point, right: Point) -> Point {
    [(left[0] + right[0]) / 2.0, (left[1] + right[1]) / 2.0]
}

fn from_points(left: Point, right: Point) -> Point {
    [left[0] - right[0], left[1] - right[1]]
}

fn dot(left: Point, right: Point) -> f32 {
    left[0] * right[0] + left[1] * right[1]
}

fn add(left: Point, right: Point) -> Point {
    [left[0] + right[0], left[1] + right[1]]
}

fn scale(point: Point, amount: f32) -> Point {
    [point[0] * amount, point[1] * amount]
}

fn distance(left: Point, right: Point) -> f32 {
    ((left[0] - right[0]).powi(2) + (left[1] - right[1]).powi(2)).sqrt()
}

fn is_ideographic(character: char) -> bool {
    matches!(character as u32,
        0x3007
        | 0x3021..=0x3029
        | 0x3038..=0x303A
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0x20000..=0x2FA1F
        | 0x30000..=0x323AF
    )
}

fn is_east_asian_punctuation(character: char) -> bool {
    matches!(
        character,
        '。' | '，'
            | '！'
            | '？'
            | '；'
            | '：'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '《'
            | '》'
            | '、'
            | '【'
            | '】'
            | '（'
            | '）'
            | '…'
            | '—'
    )
}
