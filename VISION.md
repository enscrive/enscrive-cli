# Enscrive CLI
Enscrive CLI should be have feature parity with enscrive-developer.  We are successful when the following are true:
    1. Humans can easily install and configure enscrive CLI
    2. AI Agents, when directed by a human, can utilize all enscrive-developer API capabilities via enscrive CLI
    3. Enscrive CLI installs the full enscrive stack, optionally developers can provide an enscrive.io API key and utilize their enscrive.io subscription without having to manage their own enscrive stack locally
    4. Optionally, with an enscrive.io API key developers can use enscrive locally and sync with enscrive.io subscibed tenant

## What should work in the near-term:
Human Developer: "Critically assess [document(s)], using enscrive, run eval campaigns to create and optimize enscrive agent-voices which will work well to embed and perform neural search."
AI Agent runs the campaingn and the result is an optimized agent voice for [document(s)]

Human Developer: "Using [agent-voice(s)] ingest [document(s)] to [collection]"
AI Agent uses enscrive to ingest the documents with the appropiate, eval proven agent-voices

Human Developer: "Write code that we can use in [platform] to perform neuaral search against [collection(s)]
AI Agent produces bespoke code that allows the human developer to swivel that code into [platform]

## What should work in the long-term, perhaps with tools such as https://grepvec.com 
Human Developer: "Critically assess this code base"
AI Agent uses enscrive, perhaps via tools such as grepvec, to better understand the code base

Human Developer provides ANY prompt to AI Agent
AI Agent uses enscrive, perhaps with a new tool similar to grepvec that is more focused on providing AI agents with agency to store and retrieve any memories that the agent deems important to whatever work is ongoing in the project, and optionally to store memories globally