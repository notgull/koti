// GNU AGPL v3

// Get a list of top-level comments from the document.
const siteTable = document.getElementsByClassName("sitetable")[0];
const toplevelComments = siteTable.children.filter(comment => comment.classList.contains("comment"));

// Mark each of the toplevel comments with a particular ID.
let lastId = 0;
function markCommentWithId(comment, treeId, level) {
    last
}
