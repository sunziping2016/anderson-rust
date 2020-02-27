use std::cell::RefCell;
use std::env;
use std::fs;
use nom::{
    IResult,
    branch::alt,
    bytes::complete::tag,
    character::complete::multispace0,
    combinator::{
        all_consuming,
        map,
        opt,
        recognize,
    },
    multi::many0,
    sequence::tuple
};
use nom::bytes::complete::{take_while_m_n, take_while};
use nom::sequence::terminated;
use petgraph::{
    graph::{DefaultIx, DiGraph, NodeIndex},
    visit::EdgeRef,
};
use std::rc::Rc;
use std::collections::{HashMap, hash_map::Entry, VecDeque};

/* Parser */
#[derive(Debug)]
enum ConstraintKind {
    Addr,
    Equal,
    DerefRight,
    DerefLeft,
}

#[derive(Debug)]
struct Constraint {
    left: String,
    right: String,
    kind: ConstraintKind,
}

fn parse_identifier(input: &str) -> IResult<&str, &str> {
    recognize(tuple((
        // Head
        take_while_m_n(1, 1, |chr: char| chr.is_alphabetic()),
        // Tail
        take_while(|chr: char| chr.is_alphanumeric())
    )))(input)
}

fn parse_constraint(input: &str) -> IResult<&str, Constraint> {
    map(tuple((
        multispace0,
        alt((
            // l = &r
            map(tuple((
                parse_identifier,
                multispace0,
                tag("="),
                multispace0,
                tag("&"),
                multispace0,
                parse_identifier
            )), |result: (&str, &str, &str, &str, &str, &str, &str)| Constraint{
                left: String::from(result.0),
                right: String::from(result.6),
                kind: ConstraintKind::Addr,
            }),
            // l = r
            map(tuple((
                parse_identifier,
                multispace0,
                tag("="),
                multispace0,
                parse_identifier
            )), |result: (&str, &str, &str, &str, &str)| Constraint{
                left: String::from(result.0),
                right: String::from(result.4),
                kind: ConstraintKind::Equal,
            }),
            // l = *r
            map(tuple((
                parse_identifier,
                multispace0,
                tag("="),
                multispace0,
                tag("*"),
                multispace0,
                parse_identifier
            )), |result: (&str, &str, &str, &str, &str, &str, &str)| Constraint{
                left: String::from(result.0),
                right: String::from(result.6),
                kind: ConstraintKind::DerefRight,
            }),
            // *l = r
            map(tuple((
                tag("*"),
                multispace0,
                parse_identifier,
                multispace0,
                tag("="),
                multispace0,
                parse_identifier
            )), |result: (&str, &str, &str, &str, &str, &str, &str)| Constraint{
                left: String::from(result.2),
                right: String::from(result.6),
                kind: ConstraintKind::DerefLeft,
            }),
        )),
        opt(tuple((
            multispace0,
            tag(";")
        )))
    )), |result: (&str, Constraint, Option<(&str, &str)>)| result.1 )(input)
}

fn parse_constraint_list(input: &str) -> IResult<&str, Vec<Constraint>> {
    all_consuming(terminated(
        many0(parse_constraint),
        multispace0,
    ))(input)
}

/* Resolver */
#[derive(Debug)]
struct ConstraintNode {
    id: String,
    pts: HashMap<String, ConstraintNodeRc>,
}

type ConstraintNodeRc = Rc<RefCell<ConstraintNode>>;

struct ConstraintGraph {
    nodes: HashMap<String, NodeIndex<DefaultIx>>,
    graph: DiGraph<ConstraintNodeRc, ()>,
}

impl ConstraintGraph {
    fn new() -> ConstraintGraph {
        ConstraintGraph{
            nodes: HashMap::new(),
            graph: DiGraph::new()
        }
    }
    fn add_node(&mut self, id: String) {
        match self.nodes.entry(id.clone()) {
            Entry::Vacant(entry) => {
                let v = Rc::new(RefCell::new(ConstraintNode{
                    id: id.clone(),
                    pts: HashMap::new()
                }));
                let idx = self.graph.add_node(v.clone());
                entry.insert(idx);
            },
            _ => ()
        }
    }
    fn init_nodes(&mut self, constraints: &Vec<Constraint>) {
        for constraint in constraints {
            self.add_node(constraint.left.clone());
            self.add_node(constraint.right.clone());
        }
    }
    fn export_dot(&self) -> String {
        let mut result = String::new();
        result.push_str("digraph {\n");
        for node_idx in self.graph.node_indices() {
            let node = self.graph[node_idx].borrow();
            result.push_str(&format!("  {} [label=\"{}\\n{{", node.id, node.id)[..]);
            let mut iter = node.pts.keys();
            match iter.next() {
                Some(v) => {
                    result.push_str(&format!("{}", v)[..]);
                    for v in iter {
                        result.push_str(&format!(",{}", v)[..]);
                    }
                },
                _ => ()
            }
            result.push_str(&format!("}}\"]\n")[..])
        }
        for edge in self.graph.edge_references() {
            let s = &self.graph[edge.source()].borrow().id[..];
            let t = &self.graph[edge.target()].borrow().id[..];
            result.push_str(&format!("  {} -> {}\n", s, t)[..])
        }
        result.push_str("}\n");
        result
    }
    fn init_basic_ptrs(&mut self, constraints: &Vec<Constraint>) {
        for constraint in constraints {
            if let ConstraintKind::Addr = constraint.kind {
                let right = self.graph[*self.nodes.get(&constraint.right).unwrap()].clone();
                let id = right.borrow().id.clone();
                self.graph[*self.nodes.get(&constraint.left).unwrap()].borrow_mut()
                    .pts.insert(id, right);
            }
        }
    }
    fn add_edge(&mut self, from: &String, to: &String) {
        let left_idx = self.nodes.get(from).unwrap();
        let right_idx = self.nodes.get(to).unwrap();
        self.graph.add_edge(*left_idx, *right_idx, ());
    }
    fn contains_edge(&self, from: &String, to: &String) -> bool {
        let left_idx = self.nodes.get(from).unwrap();
        let right_idx = self.nodes.get(to).unwrap();
        self.graph.contains_edge(*left_idx, *right_idx)
    }
    fn init_simple_edges(&mut self, constraints: &Vec<Constraint>) {
        for constraint in constraints {
            if let ConstraintKind::Equal = constraint.kind {
                self.add_edge(&constraint.right, &constraint.left);
            }
        }
    }
    fn solve_complex_edges(&mut self, constraints: &Vec<Constraint>) {
        let mut work_queue = VecDeque::new();
        for node_idx in self.graph.node_indices() {
            let node = self.graph[node_idx].borrow();
            if !node.pts.is_empty() {
                work_queue.push_back(node_idx)
            }
        }
        while !work_queue.is_empty() {
            let v_idx = work_queue.pop_front().unwrap();
            let v_ref = self.graph[v_idx].clone();
            let v = v_ref.borrow();
            for a in v.pts.values() {
                let a = a.borrow();
                for constraint in constraints {
                    if let ConstraintKind::DerefRight = constraint.kind {
                       if constraint.right == v.id && !self.contains_edge(&a.id, &constraint.left) {
                           self.add_edge(&a.id, &constraint.left);
                           work_queue.push_back(*self.nodes.get(&a.id).unwrap())
                       }
                    } else if let ConstraintKind::DerefLeft = constraint.kind {
                        if constraint.left == v.id && !self.contains_edge(&constraint.right, &a.id) {
                            self.add_edge(&constraint.right, &a.id);
                            work_queue.push_back(*self.nodes.get(&constraint.right).unwrap())
                        }
                    }
                }
            }
            for edge in self.graph.edge_references() {
                if edge.source() == v_idx {
                    let mut q = self.graph[edge.target()].borrow_mut();
                    let origin_size = q.pts.len();
                    q.pts.extend(v.pts.clone());
                    if origin_size != q.pts.len() {
                        work_queue.push_back(edge.target());
                    }
                }
            }
        }
    }
    fn solve(&mut self, constraints: &Vec<Constraint>) {
        self.init_nodes(&constraints);
        self.init_basic_ptrs(&constraints);
        self.init_simple_edges(&constraints);
        self.solve_complex_edges(&constraints);
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        println!("Usage: anderson-rust input.txt output.dot")
    }
    let input_filename = &args[1];
    let output_filename = &args[2];
    let input_content = fs::read_to_string(input_filename)
        .expect("Failed to open the input file");
    let constraints = parse_constraint_list(&input_content[..])
        .expect("Failed to parse the input file").1;
    let mut graph = ConstraintGraph::new();
    graph.solve(&constraints);
    fs::write(output_filename, graph.export_dot())
        .expect("Fail to write file")
}
