/** Short grammar notes for each tense code, written for beginners. */
export type Note = { use: string; form: string; example: [string, string][] };

const FR: Record<string, Note> = {
  pres: {
    use: "What is happening now, habits, and general truths. Also used for the near future in speech (« Je pars demain »).",
    form: "Stem + endings. -er verbs: -e, -es, -e, -ons, -ez, -ent. Many common verbs are irregular (être, avoir, aller, faire).",
    example: [["Je parle français.", "I speak French."], ["Il pleut.", "It is raining."]],
  },
  pc: {
    use: "The everyday past: a finished action at a definite moment. It is the past tense you hear most in films.",
    form: "Avoir or être in the present + past participle. Movement verbs (aller, venir, partir…) and reflexive verbs use être, and then the participle agrees.",
    example: [["J'ai mangé.", "I ate / I have eaten."], ["Elle est partie.", "She left."]],
  },
  imp: {
    use: "Background in the past: what was going on, how things were, or what used to happen again and again.",
    form: "Nous form of the present minus -ons + -ais, -ais, -ait, -ions, -iez, -aient. Only être is irregular (ét-).",
    example: [["Il faisait beau.", "The weather was nice."], ["Quand j'étais petit, je jouais dehors.", "When I was little, I used to play outside."]],
  },
  pqp: {
    use: "Something that had already happened before another past event.",
    form: "Avoir or être in the imparfait + past participle.",
    example: [["J'avais déjà mangé.", "I had already eaten."]],
  },
  ps: {
    use: "The literary past, used in books, fairy tales and formal narration. Recognise it; you rarely need to say it.",
    form: "-er verbs: -ai, -as, -a, -âmes, -âtes, -èrent. Others: -is or -us endings (il fit, il fut, il eut).",
    example: [["Il ouvrit la porte.", "He opened the door."]],
  },
  fut: {
    use: "What will happen, promises and predictions. After « quand » French uses the future where English uses the present.",
    form: "Infinitive (drop the final -e of -re verbs) + -ai, -as, -a, -ons, -ez, -ont. Irregular stems: ser-, aur-, ir-, fer-, pourr-, voudr-.",
    example: [["Je t'appellerai.", "I will call you."], ["Quand tu seras là…", "When you are here…"]],
  },
  fp: {
    use: "The near future: what is about to happen or is planned. Very common in speech.",
    form: "Aller in the present + infinitive.",
    example: [["Je vais partir.", "I am going to leave."]],
  },
  cond: {
    use: "Would: polite requests, wishes, and what would happen if… Also reported future in the past.",
    form: "Future stem + imparfait endings: -ais, -ais, -ait, -ions, -iez, -aient.",
    example: [["Je voudrais un café.", "I would like a coffee."], ["Si j'avais le temps, je viendrais.", "If I had time, I would come."]],
  },
  subj: {
    use: "After expressions of wanting, feeling, necessity or doubt, usually after « que » (il faut que, je veux que, bien que).",
    form: "Ils form of the present minus -ent + -e, -es, -e, -ions, -iez, -ent. Irregular: être (sois), avoir (aie), aller (aille), faire (fasse).",
    example: [["Il faut que tu viennes.", "You have to come."], ["Je veux qu'il parte.", "I want him to leave."]],
  },
  imper: {
    use: "Commands and requests. Only tu, nous and vous forms, with no subject pronoun.",
    form: "Present tense forms; -er verbs drop the -s of the tu form (parle !).",
    example: [["Attends !", "Wait!"], ["Allons-y.", "Let's go."]],
  },
  inf: {
    use: "The basic form: after another verb, after prepositions, and in instructions.",
    form: "Ends in -er, -ir or -re.",
    example: [["Je veux partir.", "I want to leave."]],
  },
  pp: {
    use: "Past participle: builds compound tenses and works like an adjective.",
    form: "-er → -é, -ir → -i, -re → -u, plus irregulars (fait, pris, mis, eu, été).",
    example: [["C'est fini.", "It's finished."]],
  },
  ppr: {
    use: "Present participle (-ing). With « en » it means while or by doing something.",
    form: "Nous form minus -ons + -ant.",
    example: [["En marchant…", "While walking…"]],
  },
};

const ES: Record<string, Note> = {
  pres: {
    use: "What happens now, habits, and often the near future.",
    form: "-ar: -o, -as, -a, -amos, -áis, -an. -er/-ir: -o, -es, -e, -emos/-imos, -éis/-ís, -en. Many stems change (quiero, puedo, pido).",
    example: [["Hablo español.", "I speak Spanish."], ["Mañana salgo.", "I'm leaving tomorrow."]],
  },
  pc: {
    use: "Pretérito perfecto: in Spain this is the everyday past for anything that happened today or in a period that is not over (hoy, esta semana, ya, nunca).",
    form: "Haber in the present (he, has, ha, hemos, habéis, han) + participle (-ado, -ido).",
    example: [["Hoy he comido tarde.", "I ate late today."], ["¿Has visto esto?", "Have you seen this?"]],
  },
  ps: {
    use: "Pretérito indefinido: a finished action at a definite past time (ayer, el año pasado). The main storytelling past.",
    form: "-ar: -é, -aste, -ó, -amos, -asteis, -aron. -er/-ir: -í, -iste, -ió, -imos, -isteis, -ieron. Irregular: fui, tuve, hice, dije.",
    example: [["Ayer hablé con ella.", "Yesterday I talked to her."]],
  },
  imp: {
    use: "Background in the past: what was going on, how things were, what used to happen.",
    form: "-ar: -aba, -abas, -aba, -ábamos, -abais, -aban. -er/-ir: -ía, -ías, -ía, -íamos, -íais, -ían. Only ser, ir and ver are irregular.",
    example: [["Cuando era niño, vivía en Madrid.", "When I was a child, I lived in Madrid."]],
  },
  pqp: {
    use: "Something that had already happened before another past event.",
    form: "Haber in the imperfect (había…) + participle.",
    example: [["Ya había salido.", "He had already left."]],
  },
  fut: {
    use: "What will happen. Also guesses about the present (« Estará en casa » = he's probably at home).",
    form: "Infinitive + -é, -ás, -á, -emos, -éis, -án. Irregular stems: tendr-, dir-, har-, podr-, sabr-, saldr-, vendr-, querr-.",
    example: [["Te llamaré.", "I will call you."]],
  },
  fp: {
    use: "The near future: what is going to happen. Very common in speech.",
    form: "Ir in the present + a + infinitive.",
    example: [["Voy a comer.", "I'm going to eat."]],
  },
  cond: {
    use: "Would: polite requests, wishes, and what would happen if…",
    form: "Future stem + -ía, -ías, -ía, -íamos, -íais, -ían.",
    example: [["Me gustaría ir.", "I would like to go."]],
  },
  subj: {
    use: "After wanting, feeling, necessity, doubt and many conjunctions (que, cuando for the future, para que, ojalá).",
    form: "Swap the vowel: -ar verbs take -e endings, -er/-ir verbs take -a endings, from the yo stem (tenga, haga, diga).",
    example: [["Quiero que vengas.", "I want you to come."], ["Cuando llegues, llámame.", "When you arrive, call me."]],
  },
  imper: {
    use: "Commands. In Spain the vosotros command ends in -d (¡Venid!, ¡Mirad!).",
    form: "Tú: like the él form of the present (habla). Vosotros: infinitive with -r → -d. Usted, nosotros and negative commands use the subjunctive.",
    example: [["¡Mira!", "Look!"], ["No te preocupes.", "Don't worry."]],
  },
  inf: {
    use: "The basic form: after another verb, after prepositions, and as a noun.",
    form: "Ends in -ar, -er or -ir.",
    example: [["Quiero salir.", "I want to go out."]],
  },
  pp: {
    use: "Past participle: builds the perfect tenses and works like an adjective.",
    form: "-ar → -ado, -er/-ir → -ido, plus irregulars (hecho, dicho, visto, puesto, escrito, vuelto).",
    example: [["Está cerrado.", "It's closed."]],
  },
  ppr: {
    use: "Gerund (-ing). With estar it describes something in progress.",
    form: "-ar → -ando, -er/-ir → -iendo (yendo, leyendo, durmiendo).",
    example: [["Estoy comiendo.", "I'm eating."]],
  },
};

const NOTES: Record<string, Record<string, Note>> = { fr: FR, es: ES };

export function grammarNote(lang: string, code: string): Note | null {
  return NOTES[lang]?.[code] ?? null;
}

/** Row titles for conjugation tables. */
export const CONJ_ROWS: Record<string, Record<string, string>> = {
  fr: {
    pres: "Présent", pc: "Passé composé", imp: "Imparfait", ps: "Passé simple", fut: "Futur", cond: "Conditionnel",
    subj: "Subjonctif", "subj-imp": "Subjonctif imparfait", imper: "Impératif",
  },
  es: {
    pres: "Presente", pc: "Pretérito perfecto", imp: "Imperfecto", ps: "Indefinido", fut: "Futuro", cond: "Condicional",
    subj: "Subjuntivo", "subj-imp": "Subjuntivo imperfecto", imper: "Imperativo",
  },
};
export const CONJ_PERSONS: Record<string, string[]> = {
  fr: ["je", "tu", "il / elle", "nous", "vous", "ils / elles"],
  es: ["yo", "tú", "él / ella", "nosotros", "vosotros", "ellos / ellas"],
  it: ["io", "tu", "lui / lei", "noi", "voi", "loro"],
  pt: ["eu", "tu", "ele / ela", "nós", "vós", "eles / elas"],
  de: ["ich", "du", "er / sie", "wir", "ihr", "sie"],
};
/** Rows that matter most to a beginner are shown first; the rest fold away. */
export const CONJ_CORE = ["pres", "pc", "imp", "fut", "cond", "subj"];
/** Which tense code in the subtitles a table row corresponds to. */
export const ROW_TENSE: Record<string, string> = { pres: "pres", pc: "pc", imp: "imp", ps: "ps", fut: "fut", cond: "cond", subj: "subj", imper: "imper" };
