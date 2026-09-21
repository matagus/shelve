# VHS tape: demo GIF for shelve (CSV pretty-printer grouped by column)
# https://github.com/charmbracelet/vhs
Set TypingSpeed 26ms
Set FontSize 15
Set Width 1400
Set Height 780
Set Theme "Dark Pastel"

# clean intro
Hide
Type "clear"
Enter
Show

# 0. what we're working with
Type "head sample-files/tasks.csv"
Enter
Sleep 2200ms

# 1. version
Type "clear"
Enter
Type "shelve --version"
Enter
Sleep 1000ms

# 2. stdin + group by Status
Type "clear"
Enter
Type "echo 'GROUP BY STATUS (col 3), via stdin:'"
Enter
Sleep 900ms
Type "cat sample-files/tasks.csv | shelve -c 3"
Enter
Sleep 3000ms

# 3. group by Priority via file argument
Type "clear"
Enter
Type "echo 'GROUP BY PRIORITY (col 5), via file arg:'"
Enter
Sleep 900ms
Type "shelve -c 5 sample-files/tasks.csv"
Enter
Sleep 3000ms

# 4. group by Assignee on a second dataset
Type "clear"
Enter
Type "echo '2nd CSV: more-tasks.csv. GROUP BY ASSIGNEE (col 4):'"
Enter
Sleep 1000ms
Type "shelve -c 4 sample-files/more-tasks.csv"
Enter
Sleep 3000ms

# 5. multiple files grouped together
Type "clear"
Enter
Type "echo 'GROUP ACROSS MULTIPLE FILES AT ONCE:'"
Enter
Sleep 900ms
Type "shelve -c 3 sample-files/tasks.csv sample-files/more-tasks.csv"
Enter
Sleep 3000ms
