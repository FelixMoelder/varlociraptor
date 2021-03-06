// Copyright 2016-2019 Johannes Köster, David Lähnemann.
// Licensed under the GNU GPLv3 license (https://opensource.org/licenses/GPL-3.0)
// This file may not be copied, modified, or distributed
// except according to those terms.

#[derive(Debug, Clone)]
pub struct TumorNormalPair<T> {
    pub tumor: T,
    pub normal: T,
}

impl<T> Into<Vec<T>> for TumorNormalPair<T> {
    fn into(self) -> Vec<T> {
        vec![self.tumor, self.normal]
    }
}

pub trait TumorNormalPairView<T> {
    fn tumor(&self) -> &T;

    fn normal(&self) -> &T;
}

impl<T> TumorNormalPairView<T> for Vec<T> {
    fn tumor(&self) -> &T {
        &self[0]
    }

    fn normal(&self) -> &T {
        &self[1]
    }
}
