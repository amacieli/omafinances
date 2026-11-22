Weekly financial report specification
-------------------------------------

Sender: adam.macielinski@gmail.com 
To: [reports] recipient in settings.toml (not in the repo)
Subject line: OmaFinances - Weekly Finances Report
Body: use the same Nerd fixed-point font for the entire message

Number formatting - for each column, right-justify all the numbers.  Do not include pennies; round to the nearest dollar.
Commas for thousands delimiter.  The biggest number in the column gets a space to the left of it, and a $ sign immediately
to the left of that.  That dollar sign will drive the location of the dollar sign for all numbers above and below
the biggest number, in that same column.

Email format.  HTML.  Use the Nerd fixed-point font for the entire message.  Put any underlining using hyphens on the next line.

Credit accounts
* Each card show in a separate row, by bank by account type
* Show HELOC as well
* First column: bank name (printed once per bank)
* Second column: card nickname (accounts.nickname); if none is set, the card's formal name
* Third column: M+0 balance (M = current month, latest, put the as-of date in the line underneath the column title)
* Fourth column: M-1 balance (put the as-of date in the line underneath the column title)
* Fifth column: M-2 balance (put the as-of date in the line underneath the column title)

Checking and saving balances 
* Same format as for credit accounts 

Transactions of focus 
* List dates, amounts, vendors, categories, notes 
* Scope is prior 90 day period from reporting date
* Anything involving Amazon in descending order of value 
* Anything involving food or eating out or casual dining, broken down by vendor by date

