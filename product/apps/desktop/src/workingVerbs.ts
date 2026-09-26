/** Presentation-only vocabulary. Keep words short; never imply success, approval,
 * deployment or a particular Git operation. Pools follow observed activity,
 * never guesses from the user's prompt. Add/remove phrases here independently
 * of the animation and native protocol. */
export const WORKING_VERBS = {
  thinking: [
    "Noodling", "Mulling", "Cogitating", "Scheming", "Reckoning", "Deliberating",
    "Untangling", "Connecting dots", "Brainstorming", "Hypothesising", "Modelling",
    "Puzzling", "Ruminating", "Contemplating", "Musing", "Pondering", "Reflecting",
    "Turning the gears", "Chewing it over", "Letting it marinate", "Plotting a course",
    "Following a hunch", "Chasing a thought", "Joining the dots", "Wrestling with it",
    "Thinking it through", "Teasing it apart", "Looking for an angle", "Making connections",
    "Mulling it over", "Working backwards", "Turning it over", "Reasoning", "Percolating",
  ],
  reading: [
    "Perusing", "Skimming", "Rummaging", "Peering inside", "Sifting through", "Unpacking",
    "Spelunking", "Deciphering", "Absorbing", "Taking it in", "Leafing through",
    "Combing through", "Reading between lines", "Exploring", "Surveying", "Taking stock",
    "Getting acquainted", "Looking under the hood", "Poking around", "Scouting",
    "Following the trail", "Dusting off", "Turning pages", "Lifting the lid",
    "Having a look", "Eyeing the details", "Browsing", "Studying", "Inspecting",
    "Getting the lay of things", "Unrolling the map", "Following the thread",
  ],
  searching: [
    "Hunting", "Trawling", "Prospecting", "Sniffing around", "Tracking down", "Digging",
    "Fossicking", "Combing through", "Casting a net", "Looking around", "Chasing leads",
    "Following clues", "Seeking", "Scouring", "Sleuthing", "Nosing around",
    "Rooting around", "Checking the corners", "Panning for gold", "Sifting the haystack",
    "Looking for needles", "Going down the rabbit hole", "Turning over stones",
    "Widening the net", "Fishing for clues", "Tracing leads", "On the lookout",
    "Hunting for breadcrumbs", "Searching", "Scoping things out",
  ],
  writing: [
    "Composing", "Drafting", "Weaving", "Shaping words", "Putting pen to paper",
    "Scrawling", "Scribbling", "Etching", "Authoring", "Sketching", "Spinning a thread",
    "Laying it out", "Putting it together", "Finding the words", "Hammering it out",
    "Filling the page", "Getting it down", "Writing it up", "Stringing it together",
    "Laying down lines", "Making a mark", "Painting with words", "Knitting it together",
    "Assembling the pieces", "Bringing it together", "Crafting", "Writing", "Inking",
  ],
  editing: [
    "Tinkering", "Stitching", "Mending", "Patching", "Tweaking", "Reshaping", "Sculpting",
    "Wrangling", "Sanding", "Buffing", "Finessing", "Reworking", "Splicing", "Pruning",
    "Tidying", "Reshuffling", "Nudging into shape", "Rejigging", "Fine-tuning",
    "Turning the screws", "Tightening things up", "Moving the pieces", "Trimming",
    "Smoothing the edges", "Giving it a polish", "Adjusting", "Tuning up", "Refining",
    "Straightening out", "Making alterations", "Doing some tailoring", "Kneading",
  ],
  building: [
    "Baking", "Forging", "Smelting", "Conjuring", "Assembling", "Fusing", "Tempering",
    "Distilling", "Casting", "Cooking it up", "Firing the kiln", "Warming the forge",
    "Turning up the heat", "Working the anvil", "Mixing the ingredients", "Building",
    "Bringing it to life", "Putting pieces together", "Making something", "In the workshop",
    "Cooking with gas", "Stacking the bricks", "Raising the frame", "Working the recipe",
  ],
  testing: [
    "Probing", "Poking", "Checking", "Exercising", "Bug-hunting", "Kicking the tyres",
    "Giving it a whirl", "Taking it for a spin", "Trying it out", "Shaking it down",
    "Checking the seams", "Looking for cracks", "Rattling the cage", "Tugging the threads",
    "Testing the waters", "Checking the wiring", "Poking the edges", "Challenging",
    "Checking the fit", "Turning it over", "Walking the checks", "Doing the rounds",
    "Putting it to work", "Checking the corners", "Trying the handles", "Testing",
  ],
  executing: [
    "Cranking", "Pulling levers", "Sending it", "Kicking off", "Making moves",
    "Working the gears", "Letting it rip", "Getting it rolling", "Setting it in motion",
    "Turning the crank", "Running the machinery", "Doing the legwork", "Making it happen",
    "Working away", "Rolling along", "Getting on with it", "Taking action",
    "Putting things in motion", "Keeping it moving", "Working the controls",
    "Running the play", "Giving it a go", "Getting stuck in", "Operating",
  ],
  general: [
    "Cooking", "Doing the thing", "Vibing", "Figuring it out", "Making headway",
    "Working on it", "Chipping away", "Getting into it", "Finding a way", "Keeping at it",
    "Getting things together", "Moving things along", "Getting organised", "Gearing up",
    "Rolling up the sleeves", "Putting things in order", "Getting situated", "Setting the stage",
    "Lining things up", "Warming the braincells", "Gathering the threads", "Taking shape",
    "Working some magic", "Making preparations",
  ],
} as const;
export type WorkingVerbKind = keyof typeof WORKING_VERBS;
export function pickWorkingVerb(kind: WorkingVerbKind, seed: string): string {
  let hash = 2166136261;
  for (let i = 0; i < seed.length; i++) hash = Math.imul(hash ^ seed.charCodeAt(i), 16777619) >>> 0;
  const words = WORKING_VERBS[kind];
  return words[hash % words.length];
}
