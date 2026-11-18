// Poll: a quick vote. Shows the two other ways a button names its handler: a function
// pointer with an argument curried in, and a closure.

fn render(settings, host) {
    let votes = this.votes ?? #{};
    let choices = settings.choices.split(",");
    let buttons = [];
    let counts = [];
    for c in choices {
        c.trim();
        buttons.push(ui::button(c, Fn("vote").curry(c)));
        counts.push(ui::text(`${c}: ${votes[c] ?? 0}`));
    }
    let shown = [ui::heading(settings.question), ui::row(#{ gap: 4 }, buttons)];
    shown += counts;
    shown.push(ui::divider());
    shown.push(ui::button("Start over", || { this.votes = #{}; }));
    ui::column(#{ gap: 8 }, shown)
}

fn vote(choice) {
    if this.votes == () { this.votes = #{}; }
    this.votes[choice] = (this.votes[choice] ?? 0) + 1;
}

#{
    name: "Poll",
    settings: [
        #{ key: "question", label: "Question", kind: "text", value: "Tea or coffee?" },
        #{ key: "choices", label: "Choices (comma between)", kind: "text", value: "Coffee, Tea" },
    ],
}
