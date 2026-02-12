# Briefing Tool: Smart Brevity Summary Redesign

## What We're Changing

Replacing the current 5-bullet-point article summaries with **Axios-style Smart Brevity** format. The AI now produces structured editorial analysis instead of generic extraction — better for podcast prep because you get the angle, significance, and context at a glance.

## Two Formats (AI Auto-Detects)

### Editorial Format (news, policy, industry events)

```
What's happening: One strong lede sentence capturing the core news.
Why it matters: 1-2 sentences explaining significance.
The big picture: Broader industry/societal implications (optional).
"Quote text" -- Speaker Name (optional)
```

**Example — EU AI Regulation:**

> **What's happening:** The European Union has passed the AI Act, the world's first comprehensive legal framework regulating artificial intelligence systems.
>
> **Why it matters:** Companies operating in Europe must now classify their AI systems by risk level and comply with strict transparency requirements, with fines up to 7% of global revenue for violations.
>
> **The big picture:** This positions Europe as the global standard-setter for AI governance, much as GDPR did for data privacy.
>
> *"This is a historic moment for digital regulation"* -- Thierry Breton

### Product Format (hardware, software, apps)

```
The product: What it is and what it does.
Cost: Pricing details (if mentioned).
Availability: When and where (if mentioned).
Platforms: What it runs on (software/apps only).
"Quote text" -- Speaker Name (optional)
```

**Example — MacBook Air M5:**

> **The product:** The new MacBook Air with M5 chip delivers 40% faster CPU performance and 18 hours of battery life in the same thin wedge design.
>
> **Cost:** Starting at $1,099 for the 13-inch model and $1,299 for the 15-inch.
>
> **Availability:** Available for preorder now, shipping March 7.
>
> **Platforms:** Runs macOS Sequoia 15.3.
>
> *"This is the best laptop for most people"* -- Tim Cook

## How It Works

1. **collect-stories** fetches articles from Raindrop.io bookmarks
2. Each article is sent to Claude AI, which auto-detects whether it's a product or editorial piece
3. AI returns the appropriate Smart Brevity format
4. Output is an org-mode file you edit in Emacs (reorder stories, tweak text, remove stories)
5. **prepare-briefing** converts the edited file to HTML for use during the show

## Technical Details

- **AI Model:** Claude 3.5 Haiku (fast, cheap — ~$0.25/run for 50 articles)
- **Fallback:** If AI can't determine format, defaults to editorial
- **Optional fields:** "The big picture" (editorial) and "Platforms" (product) can be omitted when not applicable
- **Cost per run:** ~$0.25 for all three shows combined

## Show Schedule (Automated Daily at 6pm Pacific)

| Show | Airs |
|------|------|
| TWiT | Sunday |
| MacBreak Weekly | Tuesday |
| Intelligent Machines | Wednesday |
