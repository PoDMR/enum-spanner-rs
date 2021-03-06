use std::iter;

use super::super::automaton::Automaton;
use super::super::mapping::{Mapping, Marker, SpannerEnumerator};
use super::super::progress::Progress;
use super::jump::Jump;
use bit_set::BitSet;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

//  ___           _                   _ ____
// |_ _|_ __   __| | _____  _____  __| |  _ \  __ _  __ _
//  | || '_ \ / _` |/ _ \ \/ / _ \/ _` | | | |/ _` |/ _` |
//  | || | | | (_| |  __/>  <  __/ (_| | |_| | (_| | (_| |
// |___|_| |_|\__,_|\___/_/\_\___|\__,_|____/ \__,_|\__, |
//                                                  |___/

/// DAG built from the product automaton of a variable automaton and a text.
///
/// The structure allows to enumerate efficiently  all the distinct matches of
/// the input automata over the input text (polynomial preprocessing and
/// constant delay iteration).
pub struct IndexedDag<'t> {
    automaton: Automaton,
    text: &'t str,
    jump_distance: usize,
    trimming_strategy: TrimmingStrategy,
    jump: Option<Jump>,
    toggle_progress: bool,
    create_dag_time: Option<Duration>,
    trim_time: Option<Duration>,
    index_time: Option<Duration>,
}

#[derive(Eq, PartialEq, Serialize, Deserialize, Clone, Copy)]
pub enum TrimmingStrategy {
    NoTrimming,
    PartialTrimming,
    FullTrimming,
}

impl<'t> IndexedDag<'t> {
    pub fn new(
        automaton: Automaton,
        text: &str,
        jump_distance: usize,
        trimming_strategy: TrimmingStrategy,
        toggle_progress: bool,
    ) -> IndexedDag {
        IndexedDag {
            automaton,
            text,
            jump_distance,
            trimming_strategy,
            toggle_progress,
            jump: None,
            create_dag_time: None,
            trim_time: None,
            index_time: None,
        }
    }

    pub fn num_levels(&self) -> Option<usize> {
        self.jump.as_ref().map(|j| j.num_levels())
    }

    pub fn get_times(&self) -> (Option<Duration>, Option<Duration>, Option<Duration>) {
        (self.create_dag_time, self.trim_time, self.index_time)
    }

    fn next_level<'a>(&'a self, gamma: BitSet) -> NextLevelIterator<'a> {
        let adj = self.automaton.get_rev_assignations();

        // Get list of variables that are part of the level.
        // UODO: It might still be slower to just using the list of all variables in the
        // automaton?
        let mut k = BitSet::new();
        let mut expected_markers = Vec::<&'a Marker>::new();
        let mut states = gamma.clone();
        let mut new_states = gamma.clone();

        while !new_states.is_empty() {
            let source = new_states.iter().next().unwrap();
            new_states.remove(source);
            for (label, target) in &adj[source] {
                let label_id = label.get_marker().unwrap().get_id();
                if !k.contains(label_id) {
                    expected_markers.push(&label.get_marker().unwrap());
                    k.insert(label_id);
                }
                if !states.contains(*target) {
                    states.insert(*target);
                    new_states.insert(*target);
                }
            }
        }

        NextLevelIterator::explore(&self.automaton, expected_markers, gamma)
    }

    pub fn get_memory_usage(&self) -> Option<(usize, usize, usize, usize)> {
        self.jump.as_ref().map(|j| j.get_memory_usage())
    }

    pub fn get_statistics(&self) -> Option<(usize, usize, f64, usize, usize, f64)> {
        self.jump.as_ref().map(|j| j.get_statistics())
    }
}

impl<'t> SpannerEnumerator<'t> for IndexedDag<'t> {
    fn iter<'i>(&'i self) -> Box<dyn Iterator<Item = Mapping<'t>> + 'i> {
        Box::new(IndexedDagIterator::init(self))
    }

    /// Compute the index of matches of an automaton over input text.
    fn preprocess(&mut self) {
        // Compute the jump function
        let mut jump = Jump::new(
            iter::once(self.automaton.get_initial()),
            self.automaton.get_closure_for_assignations(),
            self.automaton.get_jump_states(),
            self.text.len() + 1,
            self.automaton.get_nb_states(),
            self.jump_distance,
        );

        let closure_for_assignations = self.automaton.get_closure_for_assignations().clone();

        let start_time = Instant::now();

        let chars = self.text.chars();
        let mut progress = Progress::from_iter(chars).auto_refresh(self.toggle_progress);

        while let Some(curr_char) = progress.next() {
            let adj_for_char = self.automaton.get_adj_for_char_with_closure(curr_char);
            jump.init_next_level(adj_for_char);

            if jump.is_disconnected() {
                return;
            }
        }

        self.create_dag_time = Some(start_time.elapsed());

        let start_time = Instant::now();

        if self.trimming_strategy == TrimmingStrategy::FullTrimming {
            jump.trim_last_level(&self.automaton.finals, &closure_for_assignations);
        }

        if jump.is_disconnected() {
            return;
        }

        if self.trimming_strategy != TrimmingStrategy::NoTrimming {
            let chars = self.text.chars();
            let mut level = jump.get_last_level();
            let mut progress = Progress::from_iter(chars.rev()).auto_refresh(self.toggle_progress);

            while let Some(curr_char) = progress.next() {
                let rev_adj_for_char = self.automaton.get_rev_adj_for_char_with_closure(curr_char);
                jump.trim_level(level, rev_adj_for_char);
                level -= 1;
            }
        }

        self.trim_time = Some(start_time.elapsed());
        let start_time = Instant::now();
        let chars = self.text.chars();
        let mut progress = Progress::from_iter(chars).auto_refresh(self.toggle_progress);
        let mut level = 1;

        while let Some(curr_char) = progress.next() {
            let adj_for_char = self.automaton.get_adj_for_char(curr_char);
            jump.init_reach(level, curr_char, adj_for_char, &closure_for_assignations);
            level += 1;
        }

        self.index_time = Some(start_time.elapsed());

        self.jump = Some(jump);
    }
}

//  ___           _                   _
// |_ _|_ __   __| | _____  _____  __| |
//  | || '_ \ / _` |/ _ \ \/ / _ \/ _` |
//  | || | | | (_| |  __/>  <  __/ (_| |
// |___|_| |_|\__,_|\___/_/\_\___|\__,_|
//  ____
// |  _ \  __ _  __ _
// | | | |/ _` |/ _` |
// | |_| | (_| | (_| |
// |____/ \__,_|\__, |
//              |___/

struct IndexedDagIterator<'i, 't> {
    indexed_dag: &'i IndexedDag<'t>,
    stack: Vec<(usize, BitSet, Vec<(&'i Marker, usize)>)>,

    curr_level: usize,
    curr_mapping: Vec<(&'i Marker, usize)>,
    curr_next_level: NextLevelIterator<'i>,
    num_vars: usize,
}

impl<'i, 't> IndexedDagIterator<'i, 't> {
    fn init(indexed_dag: &'i IndexedDag<'t>) -> IndexedDagIterator<'i, 't> {
        IndexedDagIterator {
            indexed_dag,
            stack: match &indexed_dag.jump {
                None => Vec::new(),
                Some(j) => {
                    let mut start = j.finals().clone();
                    start.intersect_with(&indexed_dag.automaton.finals);

                    vec![(j.num_levels() - 1, start, Vec::new())]
                }
            },

            // `curr_next_level` is initialized empty, thus theses values will
            // be replaced before the first iteration.
            curr_next_level: NextLevelIterator::empty(&indexed_dag.automaton),
            curr_level: usize::default(),
            curr_mapping: Vec::default(),
            num_vars: indexed_dag.automaton.num_vars(),
        }
    }
}

impl<'i, 't> Iterator for IndexedDagIterator<'i, 't> {
    type Item = Mapping<'t>;

    fn next(&mut self) -> Option<Mapping<'t>> {
        loop {
            // First, consume curr_next_level.
            while let Some((s_p, mut new_gamma)) = self.curr_next_level.next() {
                if new_gamma.is_empty() {
                    continue;
                }

                //				print!("NLI output: ");
                //				for m in s_p.iter() {
                //					print!("{} ", m);
                //				}
                //				for q in new_gamma.iter() {
                //					print!{"q{} ", q};
                //				}
                //				println!("");

                let mut new_mapping = self.curr_mapping.clone();
                for marker in s_p {
                    new_mapping.push((
                        marker,
                        self.indexed_dag
                            .jump
                            .as_ref()
                            .unwrap()
                            .get_pos(self.curr_level),
                    ));
                }

                if self.curr_level == 0 {
                    if new_gamma.contains(self.indexed_dag.automaton.get_initial()) {
                        // Re-align level indexes with utf8 coding
                        let aligned_markers = new_mapping
                            .into_iter()
                            .map(|(marker, pos)| (marker.clone(), pos));

                        // Create the new mapping
                        return Some(Mapping::from_markers(
                            self.indexed_dag.text,
                            aligned_markers,
                            self.num_vars,
                        ));
                    }
                } else if let Some(jump_level) = self
                    .indexed_dag
                    .jump
                    .as_ref()
                    .unwrap()
                    .jump(self.curr_level, &mut new_gamma)
                {
                    self.stack.push((jump_level, new_gamma, new_mapping));
                }
            }

            // Overwise, read next element of the stack and init the new
            // `curr_next_level` before restarting the process.
            match self.stack.pop() {
                None => return None,
                Some((level, gamma, mapping)) => {
                    self.curr_level = level;
                    self.curr_mapping = mapping;
                    self.curr_next_level = self.indexed_dag.next_level(gamma)
                }
            }
        }
    }
}

//  _   _           _   _                   _
// | \ | | _____  _| |_| |    _____   _____| |
// |  \| |/ _ \ \/ / __| |   / _ \ \ / / _ \ |
// | |\  |  __/>  <| |_| |__|  __/\ V /  __/ |
// |_| \_|\___/_/\_\\__|_____\___| \_/ \___|_|
//  ___ _                 _
// |_ _| |_ ___ _ __ __ _| |_ ___  _ __
//  | || __/ _ \ '__/ _` | __/ _ \| '__|
//  | || ||  __/ | | (_| | || (_) | |
// |___|\__\___|_|  \__,_|\__\___/|_|
//

/// Explore all feasible variable associations in a level from a set of states
/// and resulting possible states reached for theses associations.
struct NextLevelIterator<'a> {
    automaton: &'a Automaton,

    /// Set of markers that can be reached in this level.
    expected_markers: Vec<&'a Marker>,

    /// Set of states we start the run from.
    gamma: BitSet,

    /// The current state of the iterator
    stack: Vec<(BitSet, BitSet, Vec<&'a Marker>)>,

    /// finished enumerating
    done: bool,

    /// the only partial mapping to return is the empty one
    almost_done: bool,
}

impl<'a> NextLevelIterator<'a> {
    /// An empty iterator.
    fn empty(automaton: &'a Automaton) -> NextLevelIterator<'a> {
        NextLevelIterator {
            stack: Vec::new(), // Initialized with an empty stack to stop iteration instantly.
            automaton,
            expected_markers: Vec::new(),
            gamma: BitSet::new(),
            done: true,
            almost_done: true,
        }
    }

    /// Start the exporation from the input set of states `gamma`.
    fn explore(
        automaton: &'a Automaton,
        expected_markers: Vec<&'a Marker>,
        gamma: BitSet,
    ) -> NextLevelIterator<'a> {
        NextLevelIterator {
            automaton,
            expected_markers: expected_markers.into_iter().collect(),
            gamma,
            stack: vec![(BitSet::new(), BitSet::new(), Vec::new())],
            done: false,
            almost_done: false,
        }
    }

    fn follow_sp_sm(&self, gamma: &BitSet, s_p: &BitSet, s_m: &BitSet) -> BitSet {
        let adj = self.automaton.get_rev_assignations();
        let num_states = self.automaton.get_nb_states();
        let mut path_set: Vec<i32> = vec![-1; num_states];
        let mut queue: Vec<_> = gamma.iter().map(|x| (x, 0)).collect();

        //		println!("follow_sp_sm({:?},{:?},{:?}): ", gamma, s_p, s_m);

        while let Some((source, num_labels)) = queue.pop() {
            //			println!("looking at ({},{})", source,num_labels);
            if path_set[source] >= num_labels {
                // we enumerate states in descending order and all (reverse) label-transitions are
                // from larger ids to smaller ids. Thus there is nothing to gain here.
                continue;
            }

            path_set[source] = num_labels;

            for (label, target) in &adj[source] {
                let label = label.get_marker().unwrap().get_id();

                if s_m.contains(label) || (path_set[*target] > num_labels) {
                    continue;
                }

                if s_p.contains(label) {
                    queue.push((*target, num_labels + 1));
                //					println!("found label {} to target {}", label, target);
                } else {
                    if path_set[*target] < num_labels {
                        //						println!("found non-sp label {} to target {}", label, target);
                        queue.push((*target, num_labels));
                    }
                }
            }
        }

        let num_labels_expected = s_p.len();

        //		println!("path_set: {:?}", path_set);

        let result = path_set
            .iter()
            .enumerate()
            .filter_map(|(vertex, num_labels)| match num_labels {
                &num if num >= num_labels_expected as i32 => Some(vertex),
                _ => None,
            })
            .collect();

        //		println!("{:?}", result);

        result
    }
}

impl<'a> Iterator for NextLevelIterator<'a> {
    type Item = (Vec<&'a Marker>, BitSet);

    fn next(&mut self) -> Option<(Vec<&'a Marker>, BitSet)> {
        if self.done {
            return None;
        }

        if self.almost_done {
            let mut markers = Vec::new();
            let adj = self.automaton.get_rev_assignations();
            let mut gamma2 = BitSet::new();
            let marker = self.expected_markers[0];
            let gamma = &self.gamma;

            markers.push(marker);

            for source in gamma.iter() {
                for (_, target) in &adj[source] {
                    gamma2.insert(*target);
                }
            }

            self.done = true;

            return Some((markers, gamma2));
        }

        if self.expected_markers.len() <= 1 {
            self.almost_done = true;
            if self.expected_markers.is_empty() {
                self.done = true;
            }
            return Some((Vec::new(), self.gamma.clone()));
        }

        while let Some((mut s_p, mut s_m, mut markers)) = self.stack.pop() {
            let mut gamma2 = Some(self.follow_sp_sm(&self.gamma, &s_p, &s_m));

            if gamma2.as_ref().unwrap().is_empty() {
                continue;
            }

            while s_p.len() + s_m.len() < self.expected_markers.len() {
                let depth = s_p.len() + s_m.len();
                let next_marker = self.expected_markers[depth].get_id();
                s_m.insert(next_marker);
                gamma2 = Some(self.follow_sp_sm(&self.gamma, &s_p, &s_m));

                if !gamma2.as_ref().unwrap().is_empty() {
                    // If current pair Sp/Sm is feasible, add the other branch
                    // to the stack.
                    let mut new_s_m = s_m.clone();
                    let mut new_s_p = s_p.clone();
                    new_s_p.insert(next_marker);
                    new_s_m.remove(next_marker);
                    let mut new_markers = markers.clone();
                    new_markers.push(&self.expected_markers[depth]);
                    self.stack.push((new_s_p, new_s_m, new_markers));
                } else {
                    // Overwise, the other branch has to be feasible.
                    s_m.remove(next_marker);
                    s_p.insert(next_marker);
                    markers.push(&self.expected_markers[depth]);
                    gamma2 = None;
                }
            }

            let gamma2 = match gamma2 {
                None => self.follow_sp_sm(&self.gamma, &s_p, &s_m),
                Some(val) => val,
            };

            return Some((markers, gamma2));
        }

        None
    }
}
