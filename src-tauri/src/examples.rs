#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExampleSentence {
    pub sentence_index: i64,
    pub source_text: String,
    pub words: Vec<String>,
}

pub fn split_english_example_sentences(source: &str) -> Vec<ExampleSentence> {
    let mut raw_sentences = Vec::new();
    let mut current = String::new();

    for character in source.chars() {
        current.push(character);
        if matches!(character, '.' | '?' | '!' | '…' | '\n') {
            push_raw_sentence(&mut raw_sentences, &mut current);
        }
    }
    push_raw_sentence(&mut raw_sentences, &mut current);

    raw_sentences
        .into_iter()
        .enumerate()
        .filter_map(|(index, source_text)| {
            let words = tokenize_english_words(&source_text);
            if words.is_empty() || words.len() > 15 {
                return None;
            }
            Some(ExampleSentence {
                sentence_index: index as i64,
                source_text,
                words,
            })
        })
        .collect()
}

pub fn tokenize_english_words(source: &str) -> Vec<String> {
    let characters = source.chars().collect::<Vec<_>>();
    let mut words = Vec::new();
    let mut current = String::new();

    for (index, character) in characters.iter().copied().enumerate() {
        if character.is_ascii_alphabetic() {
            current.push(character.to_ascii_lowercase());
            continue;
        }

        if character == '\''
            && !current.is_empty()
            && characters
                .get(index + 1)
                .is_some_and(|next| next.is_ascii_alphabetic())
        {
            current.push(character);
            continue;
        }

        push_word(&mut words, &mut current);
    }
    push_word(&mut words, &mut current);
    words
}

fn push_raw_sentence(sentences: &mut Vec<String>, current: &mut String) {
    let sentence = current.trim().to_string();
    if !sentence.is_empty() {
        sentences.push(sentence);
    }
    current.clear();
}

fn push_word(words: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        words.push(std::mem::take(current));
    }
}

#[cfg(test)]
mod tests {
    use super::{split_english_example_sentences, tokenize_english_words};

    #[test]
    fn tokenizes_apostrophes_and_hyphens_as_designed() {
        assert_eq!(
            tokenize_english_words("Don't re-use stateful-cache."),
            vec!["don't", "re", "use", "stateful", "cache"]
        );
    }

    #[test]
    fn keeps_fifteen_words_and_discards_sixteen() {
        let fifteen = "One two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen.";
        let sixteen = "One two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen.";
        assert_eq!(split_english_example_sentences(fifteen).len(), 1);
        assert!(split_english_example_sentences(sixteen).is_empty());
    }

    #[test]
    fn splits_on_punctuation_and_newlines() {
        let sentences = split_english_example_sentences("First sentence.\nSecond sentence?");
        assert_eq!(sentences.len(), 2);
        assert_eq!(sentences[0].source_text, "First sentence.");
        assert_eq!(sentences[1].source_text, "Second sentence?");
    }

    #[test]
    fn ignores_empty_and_non_english_fragments() {
        assert!(split_english_example_sentences("中文。 123 !!!").is_empty());
    }
}
