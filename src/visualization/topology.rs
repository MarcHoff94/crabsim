use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::cell::RefCell;
use crate::flows::MaterialFlow;
use crate::materialflow_objects::Drain;

pub struct TopologyVisualizer;

impl TopologyVisualizer {
    pub fn render_network_topology(
        material_flows: &HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>,
    ) -> String {
        let mut buf = String::new();
        buf.push_str("\n╔═════════════════════════════════════════════════════════════════════════════════╗\n");
        buf.push_str("║ Network Topology                                                                ║\n");
        buf.push_str("╚═════════════════════════════════════════════════════════════════════════════════╝\n\n");

        let mut type_map: HashMap<u32, String> = HashMap::new();
        let mut successors_map: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut source_id: Option<u32> = None;

        for (id, obj) in material_flows {
            let obj_borrow = obj.borrow();
            let any = obj_borrow.as_any();
            let (obj_type, successors) = if let Some(st) = any.downcast_ref::<crate::materialflow_objects::SingleStation>() {
                let succ_ids: Vec<u32> = st.succ.iter().map(|s| *s.borrow().get_object_id()).collect();
                ("St", succ_ids)
            } else if let Some(src) = any.downcast_ref::<crate::materialflow_objects::Source>() {
                source_id = Some(*id);
                let succ_ids: Vec<u32> = src.succ.iter().map(|s| *s.borrow().get_object_id()).collect();
                ("So", succ_ids)
            } else if any.downcast_ref::<Drain>().is_some() {
                ("Dr", vec![])
            } else {
                ("??", vec![])
            };
            type_map.insert(*id, obj_type.to_string());
            successors_map.insert(*id, successors);
        }

        if let Some(src_id) = source_id {
            let mut node_column: HashMap<u32, usize> = HashMap::new();
            let mut visited = HashSet::new();
            let mut queue = VecDeque::new();
            queue.push_back((src_id, 0));

            let mut drain_ids = Vec::new();
            for id in material_flows.keys() {
                let obj = material_flows.get(id).unwrap();
                let borrowed = obj.borrow();
                let any = borrowed.as_any();
                if any.downcast_ref::<Drain>().is_some() { drain_ids.push(*id); }
            }

            while let Some((id, col)) = queue.pop_front() {
                if visited.contains(&id) { continue; }
                visited.insert(id);
                node_column.insert(id, col);
                if let Some(successors) = successors_map.get(&id) {
                    for &succ_id in successors { queue.push_back((succ_id, col + 1)); }
                }
            }

            let mut max_col = node_column.values().max().copied().unwrap_or(0);
            let mut drain_nodes = Vec::new();
            for &drain_id in &drain_ids { node_column.insert(drain_id, max_col + 1); drain_nodes.push(drain_id); }
            max_col = node_column.values().max().copied().unwrap_or(0);

            let major_branch = Self::assign_major_branches(src_id, &successors_map);
            let column_branch_nodes = Self::group_by_column_and_branch(&node_column, &major_branch);
            let column_layout = Self::build_column_layout(&column_branch_nodes, max_col);
            let (node_row, max_height) = Self::assign_rows_with_spacing(&column_layout, &column_branch_nodes, &major_branch, &drain_nodes, max_col);
            let grid = Self::build_grid(&node_column, &node_row, max_col, max_height);
            // Print into buffer using full arrow logic
            Self::print_grid_to_string(&mut buf, &grid, &type_map, &successors_map, &node_column, max_col, max_height);
        }
        buf.push('\n');
        buf
    }

    fn print_grid_to_string(
        buf: &mut String,
        grid: &[Vec<Option<u32>>],
        type_map: &HashMap<u32, String>,
        successors_map: &HashMap<u32, Vec<u32>>,
        node_column: &HashMap<u32, usize>,
        max_col: usize,
        max_height: usize,
    ) {
        use std::fmt::Write as _;
        for row in 0..max_height {
            for col in 0..=max_col {
                if let Some(id) = grid[row][col] {
                    let type_label = type_map.get(&id).map(|s| s.as_str()).unwrap_or("??");
                    let _ = write!(buf, "[{:>2}:{:2}]", type_label, id);
                } else {
                    buf.push_str("       ");
                }
                if col < max_col {
                    Self::print_arrow_to_string(buf, grid, successors_map, node_column, row, col, max_height);
                }
            }
            buf.push('\n');
            if row < max_height - 1 {
                Self::print_inter_row_connectors_to_string(buf, grid, successors_map, node_column, row, max_col, max_height);
            }
        }
    }

    fn print_arrow_to_string(
        buf: &mut String,
        grid: &[Vec<Option<u32>>],
        successors_map: &HashMap<u32, Vec<u32>>,
        node_column: &HashMap<u32, usize>,
        row: usize,
        col: usize,
        max_height: usize,
    ) {
        let mut arrow_printed = false;
        if let Some(id) = grid[row][col] {
            if let Some(successors) = successors_map.get(&id) {
                let mut next_col_successors = Vec::new();
                for &succ_id in successors {
                    if let Some(succ_col) = node_column.get(&succ_id) {
                        if *succ_col == col + 1 {
                            for check_row in 0..max_height {
                                if grid[check_row][col + 1] == Some(succ_id) {
                                    next_col_successors.push((check_row, succ_id));
                                    break;
                                }
                            }
                        }
                    }
                }
                let mut has_far_successor = false;
                for &succ_id in successors {
                    if let Some(succ_col) = node_column.get(&succ_id) {
                        if *succ_col > col + 1 { has_far_successor = true; break; }
                    }
                }
                if !next_col_successors.is_empty() {
                    if next_col_successors.len() > 1 {
                        let rows: Vec<usize> = next_col_successors.iter().map(|(r, _)| *r).collect();
                        let min_row = *rows.iter().min().unwrap();
                        let max_row = *rows.iter().max().unwrap();
                        if row == min_row {
                            buf.push_str(" ══╗ ");
                        } else if row == max_row {
                            buf.push_str("   ╚═");
                        } else if row > min_row && row < max_row {
                            buf.push_str("   ║ ");
                        } else {
                            buf.push_str("     ");
                        }
                    } else {
                        let (succ_row, succ_id) = next_col_successors[0];
                        let mut all_preds_to_succ = Vec::new();
                        for check_row in 0..max_height {
                            if let Some(check_id) = grid[check_row][col] {
                                if let Some(check_successors) = successors_map.get(&check_id) {
                                    if check_successors.contains(&succ_id) {
                                        all_preds_to_succ.push(check_row);
                                    }
                                }
                            }
                        }
                        if all_preds_to_succ.len() > 1 && succ_row == row {
                            buf.push_str(" ──→ ");
                        } else if all_preds_to_succ.len() > 1 {
                            let min_pred = *all_preds_to_succ.iter().min().unwrap();
                            let max_pred = *all_preds_to_succ.iter().max().unwrap();
                            if row < min_pred || row > max_pred {
                                buf.push_str("     ");
                            } else if row == min_pred && succ_row == row {
                                buf.push_str(" ──→ ");
                            } else if row == max_pred && row != succ_row {
                                buf.push_str(" ══╝ ");
                            } else if row > min_pred && row < max_pred {
                                buf.push_str("   ║ ");
                            } else {
                                buf.push_str("     ");
                            }
                        } else {
                            if succ_row == row {
                                buf.push_str(" ──→ ");
                            } else if succ_row > row {
                                buf.push_str(" ──╲ ");
                            } else {
                                buf.push_str(" ──╱ ");
                            }
                        }
                    }
                    arrow_printed = true;
                } else if has_far_successor {
                    buf.push_str(" ───→");
                    arrow_printed = true;
                }
            }
        }
        if !arrow_printed {
            let mut is_continuation = false;
            for prev_col in (0..=col).rev() {
                if let Some(prev_id) = grid[row][prev_col] {
                    if let Some(successors) = successors_map.get(&prev_id) {
                        for &succ_id in successors {
                            if let Some(succ_col) = node_column.get(&succ_id) {
                                if *succ_col > col {
                                    let mut succ_in_next_col_same_row = false;
                                    if *succ_col == col + 1 {
                                        for check_row in 0..max_height {
                                            if grid[check_row][*succ_col] == Some(succ_id) && check_row == row {
                                                succ_in_next_col_same_row = true;
                                                break;
                                            }
                                        }
                                    }
                                    if !succ_in_next_col_same_row { is_continuation = true; break; }
                                }
                            }
                        }
                    }
                    if is_continuation { break; }
                }
            }
            if is_continuation { buf.push_str("────→"); } else { buf.push_str("     "); }
        }
    }

    fn print_inter_row_connectors_to_string(
        buf: &mut String,
        grid: &[Vec<Option<u32>>],
        successors_map: &HashMap<u32, Vec<u32>>,
        node_column: &HashMap<u32, usize>,
        row: usize,
        max_col: usize,
        max_height: usize,
    ) {
        for col in 0..=max_col {
            let mut needs_vertical = false;
            for check_row in 0..=row {
                if let Some(id) = grid[check_row][col] {
                    if let Some(successors) = successors_map.get(&id) {
                        let mut next_col_successors = Vec::new();
                        for &succ_id in successors {
                            if let Some(succ_col) = node_column.get(&succ_id) {
                                if *succ_col == col + 1 {
                                    for r in 0..max_height {
                                        if grid[r][col + 1] == Some(succ_id) { next_col_successors.push(r); break; }
                                    }
                                }
                            }
                        }
                        if next_col_successors.len() > 1 {
                            let min_r = *next_col_successors.iter().min().unwrap();
                            let max_r = *next_col_successors.iter().max().unwrap();
                            if check_row == min_r && row >= min_r && row < max_r { needs_vertical = true; break; }
                        }
                    }
                }
            }
            buf.push_str("       ");
            if col < max_col { if needs_vertical { buf.push_str("   ║ "); } else { buf.push_str("     "); } }
        }
        buf.push('\n');
    }
    pub fn print_network_topology(
        material_flows: &HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>,
    ) {
        // Avoid printing topology during tests to keep outputs clean
        if cfg!(test) { return; }
        println!("\n╔═════════════════════════════════════════════════════════════════════════════════╗");
        println!("║ Network Topology                                                                ║");
        println!("╚═════════════════════════════════════════════════════════════════════════════════╝\n");

        let mut type_map: HashMap<u32, String> = HashMap::new();
        let mut successors_map: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut source_id: Option<u32> = None;

        // Collect topology information
        for (id, obj) in material_flows {
            let obj_borrow = obj.borrow();
            let any = obj_borrow.as_any();
            
            let (obj_type, successors) = if let Some(st) = any.downcast_ref::<crate::materialflow_objects::SingleStation>() {
                let succ_ids: Vec<u32> = st.succ.iter()
                    .map(|s| *s.borrow().get_object_id())
                    .collect();
                ("St", succ_ids)
            } else if let Some(src) = any.downcast_ref::<crate::materialflow_objects::Source>() {
                source_id = Some(*id);
                let succ_ids: Vec<u32> = src.succ.iter()
                    .map(|s| *s.borrow().get_object_id())
                    .collect();
                ("So", succ_ids)
            } else if any.downcast_ref::<Drain>().is_some() {
                ("Dr", vec![])
            } else {
                ("??", vec![])
            };
            
            type_map.insert(*id, obj_type.to_string());
            successors_map.insert(*id, successors);
        }

        // Assign columns by computing distance from source
        if let Some(src_id) = source_id {
            let mut node_column: HashMap<u32, usize> = HashMap::new();
            let mut visited = HashSet::new();
            let mut queue = VecDeque::new();
            queue.push_back((src_id, 0));
            
            // Identify drain nodes
            let mut drain_ids = Vec::new();
            for id in material_flows.keys() {
                let obj = material_flows.get(id).unwrap();
                let borrowed = obj.borrow();
                let any = borrowed.as_any();
                if any.downcast_ref::<Drain>().is_some() {
                    drain_ids.push(*id);
                }
            }
            
            while let Some((id, col)) = queue.pop_front() {
                if visited.contains(&id) {
                    continue;
                }
                visited.insert(id);
                
                // Don't add to columns yet - we'll rebuild it with branch awareness
                node_column.insert(id, col);
                
                if let Some(successors) = successors_map.get(&id) {
                    for &succ_id in successors {
                        queue.push_back((succ_id, col + 1));
                    }
                }
            }
            
            // Move all drains to the rightmost column and assign them to row 0
            let mut max_col = node_column.values().max().copied().unwrap_or(0);
            let mut drain_nodes = Vec::new();
            for &drain_id in &drain_ids {
                // Move drain to last column + 1
                node_column.insert(drain_id, max_col + 1);
                drain_nodes.push(drain_id);
            }
            
            // Update max_col after moving drains
            max_col = node_column.values().max().copied().unwrap_or(0);
            
            // Assign nodes to branches to prevent overlap
            let _node_branch = Self::assign_branches(src_id, &successors_map);
            let major_branch = Self::assign_major_branches(src_id, &successors_map);
            
            // Rebuild columns grouped by major branch
            let column_branch_nodes = Self::group_by_column_and_branch(
                &node_column,
                &major_branch,
            );
            
            let column_layout = Self::build_column_layout(
                &column_branch_nodes,
                max_col,
            );
            
            // Assign rows with proper spacing between branches
            let (node_row, max_height) = Self::assign_rows_with_spacing(
                &column_layout,
                &column_branch_nodes,
                &major_branch,
                &drain_nodes,
                max_col,
            );
            
            // Build 2D grid
            let grid = Self::build_grid(&node_column, &node_row, max_col, max_height);
            
            // Print the grid with arrows
            Self::print_grid(&grid, &type_map, &successors_map, &node_column, max_col, max_height);
        }
        println!();
    }

    fn assign_branches(
        src_id: u32,
        successors_map: &HashMap<u32, Vec<u32>>,
    ) -> HashMap<u32, usize> {
        let mut node_branch: HashMap<u32, usize> = HashMap::new();
        let mut branch_counter = 0usize;
        
        let mut branch_queue = VecDeque::new();
        let mut branch_visited = HashSet::new();
        branch_queue.push_back((src_id, 0usize));
        
        while let Some((id, branch_id)) = branch_queue.pop_front() {
            if branch_visited.contains(&id) {
                continue;
            }
            branch_visited.insert(id);
            node_branch.insert(id, branch_id);
            
            if let Some(successors) = successors_map.get(&id) {
                if successors.len() > 1 {
                    for &succ_id in successors {
                        branch_counter += 1;
                        branch_queue.push_back((succ_id, branch_counter));
                    }
                } else {
                    for &succ_id in successors {
                        branch_queue.push_back((succ_id, branch_id));
                    }
                }
            }
        }
        
        node_branch
    }

    fn assign_major_branches(
        src_id: u32,
        successors_map: &HashMap<u32, Vec<u32>>,
    ) -> HashMap<u32, usize> {
        let mut major_branch: HashMap<u32, usize> = HashMap::new();
        let mut first_branch_node = None;
        
        let mut bfs_queue = VecDeque::new();
        let mut bfs_visited = HashSet::new();
        bfs_queue.push_back(src_id);
        
        while let Some(node_id) = bfs_queue.pop_front() {
            if bfs_visited.contains(&node_id) {
                continue;
            }
            bfs_visited.insert(node_id);
            
            if let Some(successors) = successors_map.get(&node_id) {
                if successors.len() > 1 {
                    first_branch_node = Some(node_id);
                    break;
                }
                for &succ in successors {
                    bfs_queue.push_back(succ);
                }
            }
        }
        
        if let Some(branch_node) = first_branch_node {
            if let Some(branch_successors) = successors_map.get(&branch_node) {
                for (major_idx, &major_succ) in branch_successors.iter().enumerate() {
                    let mut desc_queue = VecDeque::new();
                    let mut desc_visited = HashSet::new();
                    desc_queue.push_back(major_succ);
                    
                    while let Some(desc_id) = desc_queue.pop_front() {
                        if desc_visited.contains(&desc_id) {
                            continue;
                        }
                        desc_visited.insert(desc_id);
                        major_branch.insert(desc_id, major_idx);
                        
                        if let Some(desc_successors) = successors_map.get(&desc_id) {
                            for &desc_succ in desc_successors {
                                desc_queue.push_back(desc_succ);
                            }
                        }
                    }
                }
            }
        }
        
        major_branch
    }

    fn group_by_column_and_branch(
        node_column: &HashMap<u32, usize>,
        major_branch: &HashMap<u32, usize>,
    ) -> HashMap<usize, HashMap<usize, Vec<u32>>> {
        let mut column_branch_nodes: HashMap<usize, HashMap<usize, Vec<u32>>> = HashMap::new();
        
        for (&node_id, &col) in node_column {
            let major = major_branch.get(&node_id).copied().unwrap_or(0);
            column_branch_nodes.entry(col)
                .or_insert_with(HashMap::new)
                .entry(major)
                .or_insert_with(Vec::new)
                .push(node_id);
        }
        
        column_branch_nodes
    }

    fn build_column_layout(
        column_branch_nodes: &HashMap<usize, HashMap<usize, Vec<u32>>>,
        max_col: usize,
    ) -> HashMap<usize, Vec<u32>> {
        let mut column_layout: HashMap<usize, Vec<u32>> = HashMap::new();
        
        for col in 0..=max_col {
            if let Some(branch_map) = column_branch_nodes.get(&col) {
                let mut sorted_branches: Vec<_> = branch_map.keys().copied().collect();
                sorted_branches.sort();
                
                let mut col_nodes = Vec::new();
                for branch_id in sorted_branches {
                    if let Some(mut nodes) = branch_map.get(&branch_id).cloned() {
                        nodes.sort();
                        col_nodes.extend(nodes);
                    }
                }
                
                column_layout.insert(col, col_nodes);
            }
        }
        
        column_layout
    }

    fn assign_rows_with_spacing(
        column_layout: &HashMap<usize, Vec<u32>>,
        column_branch_nodes: &HashMap<usize, HashMap<usize, Vec<u32>>>,
        major_branch: &HashMap<u32, usize>,
        drain_nodes: &[u32],
        max_col: usize,
    ) -> (HashMap<u32, usize>, usize) {
        let mut node_row: HashMap<u32, usize> = HashMap::new();
        
        // Find all branch IDs and calculate offsets
        let mut all_branches = HashSet::new();
        for col_branches in column_branch_nodes.values() {
            for &branch_id in col_branches.keys() {
                all_branches.insert(branch_id);
            }
        }
        
        let mut sorted_all_branches: Vec<_> = all_branches.into_iter().collect();
        sorted_all_branches.sort();
        
        let mut branch_offsets: HashMap<usize, usize> = HashMap::new();
        let mut offset = 0usize;
        
        for branch_id in sorted_all_branches {
            branch_offsets.insert(branch_id, offset);
            let mut max_in_branch = 0;
            for col in 0..=max_col {
                if let Some(col_branches) = column_branch_nodes.get(&col) {
                    if let Some(nodes) = col_branches.get(&branch_id) {
                        max_in_branch = max_in_branch.max(nodes.len());
                    }
                }
            }
            offset += max_in_branch + 1;
        }
        
        let mut max_height = 0usize;
        for col in 0..=max_col {
            if let Some(nodes) = column_layout.get(&col) {
                let mut branch_row_counter: HashMap<usize, usize> = HashMap::new();
                
                for &node_id in nodes {
                    if drain_nodes.contains(&node_id) {
                        node_row.insert(node_id, 0);
                    } else {
                        let major = major_branch.get(&node_id).copied().unwrap_or(0);
                        let base_offset = branch_offsets.get(&major).copied().unwrap_or(0);
                        let row_in_branch = branch_row_counter.entry(major).or_insert(0);
                        
                        node_row.insert(node_id, base_offset + *row_in_branch);
                        *row_in_branch += 1;
                    }
                }
                
                let col_max = node_row.values().max().copied().unwrap_or(0) + 1;
                max_height = max_height.max(col_max);
            }
        }
        
        (node_row, max_height)
    }

    fn build_grid(
        node_column: &HashMap<u32, usize>,
        node_row: &HashMap<u32, usize>,
        max_col: usize,
        max_height: usize,
    ) -> Vec<Vec<Option<u32>>> {
        let mut grid: Vec<Vec<Option<u32>>> = vec![vec![None; max_col + 1]; max_height];
        
        for (&node_id, &col) in node_column {
            if let Some(&row) = node_row.get(&node_id) {
                grid[row][col] = Some(node_id);
            }
        }
        
        grid
    }

    fn print_grid(
        grid: &[Vec<Option<u32>>],
        type_map: &HashMap<u32, String>,
        successors_map: &HashMap<u32, Vec<u32>>,
        node_column: &HashMap<u32, usize>,
        max_col: usize,
        max_height: usize,
    ) {
        for row in 0..max_height {
            // Print the nodes in this row
            for col in 0..=max_col {
                if let Some(id) = grid[row][col] {
                    let type_label = type_map.get(&id).map(|s| s.as_str()).unwrap_or("??");
                    print!("[{:>2}:{:2}]", type_label, id);
                } else {
                    print!("       ");
                }
                
                // Print arrows between columns
                if col < max_col {
                    Self::print_arrow(
                        grid,
                        successors_map,
                        node_column,
                        row,
                        col,
                        max_height,
                    );
                }
            }
            println!();
            
            // Print inter-row connectors for forks
            if row < max_height - 1 {
                Self::print_inter_row_connectors(
                    grid,
                    successors_map,
                    node_column,
                    row,
                    max_col,
                    max_height,
                );
            }
        }
    }

    fn print_arrow(
        grid: &[Vec<Option<u32>>],
        successors_map: &HashMap<u32, Vec<u32>>,
        node_column: &HashMap<u32, usize>,
        row: usize,
        col: usize,
        max_height: usize,
    ) {
        let mut arrow_printed = false;
        
        if let Some(id) = grid[row][col] {
            if let Some(successors) = successors_map.get(&id) {
                let mut next_col_successors = Vec::new();
                for &succ_id in successors {
                    if let Some(succ_col) = node_column.get(&succ_id) {
                        if *succ_col == col + 1 {
                            for check_row in 0..max_height {
                                if grid[check_row][col + 1] == Some(succ_id) {
                                    next_col_successors.push((check_row, succ_id));
                                    break;
                                }
                            }
                        }
                    }
                }
                
                let mut has_far_successor = false;
                for &succ_id in successors {
                    if let Some(succ_col) = node_column.get(&succ_id) {
                        if *succ_col > col + 1 {
                            has_far_successor = true;
                            break;
                        }
                    }
                }
                
                if !next_col_successors.is_empty() {
                    if next_col_successors.len() > 1 {
                        let rows: Vec<usize> = next_col_successors.iter().map(|(r, _)| *r).collect();
                        let min_row = *rows.iter().min().unwrap();
                        let max_row = *rows.iter().max().unwrap();
                        
                        if row == min_row {
                            print!(" ══╗ ");
                        } else if row == max_row {
                            print!("   ╚═");
                        } else if row > min_row && row < max_row {
                            print!("   ║ ");
                        } else {
                            print!("     ");
                        }
                    } else {
                        let (succ_row, succ_id) = next_col_successors[0];
                        
                        let mut all_preds_to_succ = Vec::new();
                        for check_row in 0..max_height {
                            if let Some(check_id) = grid[check_row][col] {
                                if let Some(check_successors) = successors_map.get(&check_id) {
                                    if check_successors.contains(&succ_id) {
                                        all_preds_to_succ.push(check_row);
                                    }
                                }
                            }
                        }
                        
                        if all_preds_to_succ.len() > 1 && succ_row == row {
                            print!(" ──→ ");
                        } else if all_preds_to_succ.len() > 1 {
                            let min_pred = *all_preds_to_succ.iter().min().unwrap();
                            let max_pred = *all_preds_to_succ.iter().max().unwrap();
                            
                            if row < min_pred || row > max_pred {
                                print!("     ");
                            } else if row == min_pred && succ_row == row {
                                print!(" ──→ ");
                            } else if row == max_pred && row != succ_row {
                                print!(" ══╝ ");
                            } else if row > min_pred && row < max_pred {
                                print!("   ║ ");
                            } else {
                                print!("     ");
                            }
                        } else {
                            if succ_row == row {
                                print!(" ──→ ");
                            } else if succ_row > row {
                                print!(" ──╲ ");
                            } else {
                                print!(" ──╱ ");
                            }
                        }
                    }
                    arrow_printed = true;
                } else if has_far_successor {
                    print!(" ───→");
                    arrow_printed = true;
                }
            }
        }
        
        if !arrow_printed {
            let mut is_continuation = false;
            for prev_col in (0..=col).rev() {
                if let Some(prev_id) = grid[row][prev_col] {
                    if let Some(successors) = successors_map.get(&prev_id) {
                        for &succ_id in successors {
                            if let Some(succ_col) = node_column.get(&succ_id) {
                                if *succ_col > col {
                                    let mut succ_in_next_col_same_row = false;
                                    if *succ_col == col + 1 {
                                        for check_row in 0..max_height {
                                            if grid[check_row][*succ_col] == Some(succ_id) && check_row == row {
                                                succ_in_next_col_same_row = true;
                                                break;
                                            }
                                        }
                                    }
                                    if !succ_in_next_col_same_row {
                                        is_continuation = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    if is_continuation { break; }
                }
            }
            
            if is_continuation {
                print!("────→");
            } else {
                print!("     ");
            }
        }
    }

    fn print_inter_row_connectors(
        grid: &[Vec<Option<u32>>],
        successors_map: &HashMap<u32, Vec<u32>>,
        node_column: &HashMap<u32, usize>,
        row: usize,
        max_col: usize,
        max_height: usize,
    ) {
        for col in 0..=max_col {
            let mut needs_vertical = false;
            
            for check_row in 0..=row {
                if let Some(id) = grid[check_row][col] {
                    if let Some(successors) = successors_map.get(&id) {
                        let mut next_col_successors = Vec::new();
                        for &succ_id in successors {
                            if let Some(succ_col) = node_column.get(&succ_id) {
                                if *succ_col == col + 1 {
                                    for r in 0..max_height {
                                        if grid[r][col + 1] == Some(succ_id) {
                                            next_col_successors.push(r);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        
                        if next_col_successors.len() > 1 {
                            let min_r = *next_col_successors.iter().min().unwrap();
                            let max_r = *next_col_successors.iter().max().unwrap();
                            if check_row == min_r && row >= min_r && row < max_r {
                                needs_vertical = true;
                                break;
                            }
                        }
                    }
                }
            }
            
            if needs_vertical {
                print!("       ");
                if col < max_col {
                    print!("   ║ ");
                }
            } else {
                print!("       ");
                if col < max_col {
                    print!("     ");
                }
            }
        }
        println!();
    }
}
