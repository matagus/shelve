use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::error::Error;
use std::fs::File;

#[derive(Debug)]
pub struct Row {
    data: Vec<String>,
}

#[derive(Debug)]
pub struct GroupedData {
    groups: BTreeMap<String, Vec<Row>>,
}

impl Row {
    fn new(data: Vec<String>) -> Self {
        Row { data }
    }

    pub fn join_without_col(&self, separator: &str, index: usize) -> String {
        let mut data = self.data.clone();
        data.remove(index);
        data.join(separator)
    }
}

impl GroupedData {
    fn new() -> Self {
        GroupedData {
            groups: BTreeMap::new(),
        }
    }

    fn process<R: std::io::Read>(&mut self, rdr: &mut csv::Reader<R>, index: usize) -> Result<(), Box<dyn Error>> {
        for result in rdr.records() {
            let record = result?;

            // Check if the column index is within bounds
            if index >= record.len() {
                return Err(format!("Column index {} is out of bounds", index).into());
            }

            let key = record[index].to_string();
            let row = Row::new(record.iter().map(|s| s.to_string()).collect());
            self.add(&key, row);
        }
        Ok(())
    }

    pub fn from_files(filename_vec: &[String], index: usize) -> Result<Self, Box<dyn Error>> {
        let mut groups = GroupedData::new();

        if filename_vec.is_empty() {
            let stdin = std::io::stdin().lock();
            let mut rdr = csv::Reader::from_reader(stdin);
            groups.process(&mut rdr, index)?;
        } else {
            for filename in filename_vec {
                // Create a CSV reader
                let mut rdr = csv::Reader::from_reader(File::open(filename)?);
                groups.process(&mut rdr, index)?;
            }
        }

        Ok(groups)
    }

    pub fn add(&mut self, group_name: &str, row: Row) {
        match self.groups.entry(group_name.to_string()) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().push(row);
            }
            Entry::Vacant(entry) => {
                entry.insert(vec![row]);
            }
        }
    }

    pub fn get_groups(&self) -> Vec<&String> {
        self.groups.keys().collect()
    }

    pub fn get_rows(&self, group_name: &str) -> Option<&Vec<Row>> {
        self.groups.get(group_name)
    }
}
