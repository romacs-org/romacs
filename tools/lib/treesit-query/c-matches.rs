    #[cfg(test)]
    fn matches_in(
        &self,
        _tree: &tree_sitter::Tree,
        node: Node,
        range: (u32, u32),
        stop: &crate::quit::NativeStop,
    ) -> Option<Vec<Match>> {
        self.matches(node, range, stop)
    }
