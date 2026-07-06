from cogs.changelog_publisher import SpamLearningView, _parse_spam_learning


def test_spam_learning_payload_parse_valid():
    payload = _parse_spam_learning(
        {
            "pattern": "  @x viewer bot pitch  ",
            "pattern_type": "phrase",
            "source_message": "hello\nworld",
            "source_channel": "#demo",
            "reason": "Score 1",
        }
    )

    assert payload == {
        "pattern": "@x viewer bot pitch",
        "pattern_type": "phrase",
        "source_message": "hello world",
        "source_channel": "#demo",
        "reason": "Score 1",
    }


def test_spam_learning_payload_rejects_short_pattern():
    assert _parse_spam_learning({"pattern": "abc"}) is None


def test_spam_learning_view_has_two_buttons():
    view = SpamLearningView(
        {
            "pattern": "viewer bot pitch",
            "pattern_type": "phrase",
            "source_message": "viewer bot pitch",
            "source_channel": "demo",
            "reason": "Score 1",
        }
    )

    labels = [child.label for child in view.children]
    assert labels == ["Spam lernen", "Harmlos lernen"]
